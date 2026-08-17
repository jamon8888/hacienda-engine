//! MCP (Model Context Protocol) server for hacienda.
//!
//! Exposes [`hacienda::HaciendaFacade`] to MCP clients (Claude Desktop, agent frameworks)
//! as a set of tools. See `superpowers/specs/2026-08-13-M1-mcp-server-and-cli-sdk-parity-design.md`
//! for the design this crate implements.
//!
//! # Why this is not xberg's own MCP server
//!
//! `xberg` ships its own MCP server behind its `mcp` cargo feature
//! (`crates/xberg/src/mcp/`): `extract`, `extract_batch`, `detect_mime_type`, and four
//! cache-management tools, all returning **unredacted** document content — there is no
//! PII detection or audit chain on that path, by design, because xberg is a document
//! extraction library and hacienda's PII/audit layer is what wraps it.
//!
//! hacienda never enables that feature and never imports from that module (see the root
//! `Cargo.toml`'s `xberg` dependency entry). Every tool in this crate calls through
//! [`hacienda::HaciendaFacade`] instead — the same seam `hacienda-api`'s HTTP handlers and
//! `hacienda-cli`'s subcommands already call through — so an MCP client gets exactly the
//! same redaction and audit guarantees an HTTP client gets, never xberg's raw extraction
//! output. Net xberg-crate diff for this crate: zero lines.
//!
//! # Trust boundary
//!
//! [`HaciendaMcp`] always calls the facade with `Caller::Trusted` — the same precedent
//! `hacienda-cli`'s `serve` and `pii reveal` already document: the process boundary is the
//! trust boundary. This matches the **stdio** transport this crate implements today
//! (M1a): a local MCP client (Claude Desktop, a local agent) launches `hacienda mcp serve`
//! as a child process, exactly as it would launch any other local MCP server. An HTTP/SSE
//! transport (M1b) would need to authenticate each call through the same capability
//! checks `hacienda-api`'s `auth_middleware` already enforces — that transport is not
//! implemented in this crate yet.
//!
//! # Tool inventory
//!
//! Eight tools, each a thin wrapper over one [`hacienda::HaciendaFacade`] method already
//! exercised by `hacienda-api`'s handlers or `hacienda-cli`'s subcommands:
//!
//! | Tool | Facade method | Mirrors |
//! | --- | --- | --- |
//! | `documents_process` | `process_batch_with_auth` | `POST /v1/documents`, `hacienda extract` |
//! | `pii_scan` | `scan_text_with_auth` | `POST /v1/pii/scan` |
//! | `pii_redact` | `redact_text_with_auth` | `POST /v1/pii/redact` |
//! | `pii_reveal` | `reveal_token_with_auth` | `POST /v1/pii/reveal`, `hacienda pii reveal` |
//! | `audit_verify` | `verify_audit_with_auth` | `GET /v1/audit/verify` |
//! | `audit_entries` | `audit_history_with_auth` | `GET /v1/audit/entries` |
//! | `compliance_report` | `compliance_report_with_auth` | `GET /v1/compliance/report` |
//! | `get_version` | (static) | `GET /version` |

mod params;

use std::sync::Arc;

use hacienda::HaciendaFacade;
use hacienda_core::{auth::Caller, facade::SpanText};
use params::{AuditEntriesParams, DocumentsProcessParams, EmptyParams, RevealParams, TextParams};
use rmcp::{
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::*,
    tool, tool_handler, tool_router,
    transport::stdio,
    ServerHandler, ServiceExt,
};
use xberg::ExtractInput;

/// Page size used when a tool call does not ask for one, mirroring
/// `hacienda-api/src/handlers/audit.rs`'s `DEFAULT_PAGE_LIMIT`.
const DEFAULT_PAGE_LIMIT: usize = 100;
/// Largest page this tool will serve, mirroring that module's `MAX_PAGE_LIMIT`.
const MAX_PAGE_LIMIT: usize = 1000;

/// The hacienda MCP server. Holds the facade every tool call routes through.
#[derive(Clone)]
pub struct HaciendaMcp {
    facade: Arc<HaciendaFacade>,
    // Read by the `#[tool_handler]`-generated `ServerHandler::{list_tools,call_tool}` —
    // rustc's dead-code pass does not see through that macro expansion.
    #[allow(dead_code)]
    tool_router: ToolRouter<HaciendaMcp>,
}

#[tool_router]
impl HaciendaMcp {
    /// Build a server around an already-configured facade.
    ///
    /// The caller (`hacienda-cli`'s `mcp serve` subcommand) owns config loading and
    /// facade construction — the same `HaciendaFacade` a `hacienda serve` process would
    /// build from the same configuration file, so a tool call here behaves identically to
    /// the equivalent REST call against a `serve`d instance with the same config.
    pub fn new(facade: Arc<HaciendaFacade>) -> Self {
        Self {
            facade,
            tool_router: Self::tool_router(),
        }
    }

    /// Run the server on the stdio transport until the client disconnects.
    ///
    /// This is the transport Claude Desktop and most local MCP clients use: they launch
    /// the server as a child process and speak JSON-RPC 2.0 over its stdin/stdout.
    pub async fn serve_stdio(self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let service = self.serve(stdio()).await?;
        service.waiting().await?;
        Ok(())
    }

    /// Extract and redact one or more documents.
    #[tool(
        description = "Extract text from documents (local paths or file:// URIs) and redact \
            any detected PII before returning it. Mirrors `POST /v1/documents` and `hacienda \
            extract`. Every redacted span is recorded in the audit chain.",
        annotations(
            title = "Process Documents",
            // Not read-only: it reads arbitrary local files named by the caller and
            // appends to the audit chain — the same reason `pii_redact` below is
            // `read_only_hint = false` even though it doesn't touch the filesystem.
            read_only_hint = false,
            idempotent_hint = true
        )
    )]
    async fn documents_process(
        &self,
        Parameters(params): Parameters<DocumentsProcessParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        // One `process_batch_with_auth` call for every input, not one `process_with_auth`
        // call per input — matching `POST /v1/documents`'s handler exactly
        // (`hacienda-api/src/handlers/documents.rs`). A per-input loop would give every
        // document its own audit-chain read and its own glossary/compliance snapshot,
        // and would run every document's PII detection sequentially instead of under
        // `PipelineConfig::concurrency`'s bound — this tool's own doc table claims parity
        // with the REST path, so its behaviour needs to match, not just its shape.
        let inputs: Vec<ExtractInput> = params.inputs.iter().map(ExtractInput::from_uri).collect();
        let result = self
            .facade
            .process_batch_with_auth(Caller::Trusted, inputs)
            .await
            .map_err(map_hacienda_error)?;
        json_tool_result(&result)
    }

    /// Scan raw text for PII, without rewriting it.
    #[tool(
        description = "Detect PII in raw text without redacting it. Entity text is omitted \
            from the response — only category, offsets, and confidence are returned. Mirrors \
            `POST /v1/pii/scan`.",
        annotations(
            title = "Scan Text for PII",
            read_only_hint = true,
            idempotent_hint = true
        )
    )]
    async fn pii_scan(
        &self,
        Parameters(params): Parameters<TextParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let result = self
            .facade
            .scan_text_with_auth(Caller::Trusted, &params.text, SpanText::Omit)
            .await
            .map_err(map_hacienda_error)?;
        json_tool_result(&result)
    }

    /// Redact raw text.
    #[tool(
        description = "Detect and redact PII in raw text, returning the redacted text. \
            Mirrors `POST /v1/pii/redact`. Every redacted span is recorded in the audit \
            chain.",
        annotations(title = "Redact Text", read_only_hint = false, idempotent_hint = true)
    )]
    async fn pii_redact(
        &self,
        Parameters(params): Parameters<TextParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let result = self
            .facade
            .redact_text_with_auth(Caller::Trusted, &params.text)
            .await
            .map_err(map_hacienda_error)?;
        json_tool_result(&result)
    }

    /// Reverse a pseudonym token to its plaintext.
    #[tool(
        description = "Reverse a pseudonym token (e.g. `[EMAIL:k1:base32...]`) to its \
            original plaintext. Requires the server to be configured with a pseudonymisation \
            key. Records a `Reveal` audit entry. Mirrors `POST /v1/pii/reveal` and `hacienda \
            pii reveal`.",
        annotations(
            title = "Reveal Pseudonym",
            read_only_hint = false,
            idempotent_hint = false
        )
    )]
    async fn pii_reveal(
        &self,
        Parameters(params): Parameters<RevealParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        // Collapsed, mirroring `hacienda-api/src/error.rs`'s `PseudonymError` mapping and
        // `hacienda-cli`'s `run_pii_reveal_inner`: never let the specific variant reach the
        // caller, so a probing client cannot use error text to guess at a token or key.
        let plaintext = match self
            .facade
            .reveal_token_with_auth(Caller::Trusted, &params.token)
            .await
        {
            Ok(text) => text,
            Err(hacienda::HaciendaError::Pseudonym(_) | hacienda::HaciendaError::PiiDisabled) => {
                return Err(rmcp::ErrorData::invalid_params(
                    "invalid pseudonym token",
                    None,
                ))
            }
            Err(other) => return Err(map_hacienda_error(other)),
        };
        let tip = self.facade.audit_tip().await.map_err(map_hacienda_error)?;
        json_tool_result(&serde_json::json!({ "plaintext": plaintext, "audit_chain_tip": tip }))
    }

    /// Verify the audit chain has not been tampered with.
    #[tool(
        description = "Verify this node's audit chain integrity. A broken chain is reported \
            in the result (verified: false, naming the offending entry), not as a tool error. \
            Mirrors `GET /v1/audit/verify`.",
        annotations(
            title = "Verify Audit Chain",
            read_only_hint = true,
            idempotent_hint = true
        )
    )]
    async fn audit_verify(
        &self,
        Parameters(_): Parameters<EmptyParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        if !self.facade.audit_enabled() {
            return Err(rmcp::ErrorData::invalid_params(
                "auditing is not enabled on this server",
                None,
            ));
        }
        let verdict = self.facade.verify_audit_with_auth(Caller::Trusted).await;
        let tip = self.facade.audit_tip().await.map_err(map_hacienda_error)?;
        match verdict {
            Ok(()) => json_tool_result(&serde_json::json!({
                "verified": true,
                "audit_chain_tip": tip
            })),
            Err(hacienda::HaciendaError::Audit(error)) => match classify_verify_failure(&error) {
                // A tamper-detecting variant: report it in the result body, naming the
                // offending entry, exactly like `GET /v1/audit/verify`'s
                // `classify_verify_failure` — not as a tool error, so a caller can tell
                // "the chain is broken" apart from "the check itself failed to run".
                Some(detail) => json_tool_result(&serde_json::json!({
                    "verified": false,
                    "failure": detail,
                    "audit_chain_tip": tip
                })),
                // Not a tamper verdict — an I/O or serialisation failure. Propagate it as
                // an error so it lands in the host's error rate rather than being
                // reported to a caller as tampering.
                None => Err(map_hacienda_error(hacienda::HaciendaError::Audit(error))),
            },
            Err(other) => Err(map_hacienda_error(other)),
        }
    }

    /// Read one page of the audit chain.
    #[tool(
        description = "Read one page of this node's audit history, oldest first. Page until \
            you receive an empty page. Mirrors `GET /v1/audit/entries`.",
        annotations(
            title = "Read Audit Entries",
            read_only_hint = true,
            idempotent_hint = true
        )
    )]
    async fn audit_entries(
        &self,
        Parameters(params): Parameters<AuditEntriesParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let limit = match params.limit {
            None => DEFAULT_PAGE_LIMIT,
            Some(0) => {
                return Err(rmcp::ErrorData::invalid_params(
                    "`limit` must be at least 1: a limit of 0 returns an empty page, \
                     indistinguishable from having reached the end of the history",
                    None,
                ))
            }
            Some(n) => n.min(MAX_PAGE_LIMIT),
        };
        let cursor = match params.cursor {
            None => None,
            Some(raw) => Some(
                raw.parse::<hacienda_core::audit::AuditCursor>()
                    .map_err(|_| {
                        rmcp::ErrorData::invalid_params(
                            "`cursor` is not a position in this node's audit history; use a \
                         `next_cursor` this tool previously returned",
                            None,
                        )
                    })?,
            ),
        };

        let page = self
            .facade
            .audit_history_with_auth(Caller::Trusted, cursor.as_ref(), limit)
            .await
            .map_err(map_hacienda_error)?
            .ok_or_else(|| {
                rmcp::ErrorData::invalid_params("auditing is not enabled on this server", None)
            })?;

        json_tool_result(&serde_json::json!({
            "entries": page.entries,
            "next_cursor": page.next.map(|c| c.to_string()),
        }))
    }

    /// Generate the current compliance report.
    #[tool(
        description = "Generate the compliance report (model card, DORA, checklist) for the \
            server's current pipeline configuration. Mirrors `GET /v1/compliance/report`.",
        annotations(
            title = "Compliance Report",
            read_only_hint = true,
            idempotent_hint = true
        )
    )]
    async fn compliance_report(
        &self,
        Parameters(_): Parameters<EmptyParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let report = self
            .facade
            .compliance_report_with_auth(Caller::Trusted)
            .await
            .map_err(map_hacienda_error)?
            .ok_or_else(|| {
                rmcp::ErrorData::invalid_params("compliance reporting is not configured", None)
            })?;
        json_tool_result(&report)
    }

    /// Get hacienda's version.
    #[tool(
        description = "Get the current hacienda version.",
        annotations(title = "Get Version", read_only_hint = true, idempotent_hint = true)
    )]
    fn get_version(
        &self,
        Parameters(_): Parameters<EmptyParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        json_tool_result(&serde_json::json!({ "version": env!("CARGO_PKG_VERSION") }))
    }
}

#[tool_handler]
impl ServerHandler for HaciendaMcp {
    fn get_info(&self) -> ServerInfo {
        let capabilities = ServerCapabilities::builder().enable_tools().build();

        let server_info = Implementation::new("hacienda-mcp", env!("CARGO_PKG_VERSION"))
            .with_title("Hacienda Document Intelligence MCP Server")
            .with_description(
                "Redaction-by-default document extraction, PII detection, reversible \
                 pseudonymisation, and a tamper-evident audit chain — the same pipeline \
                 hacienda's REST API and CLI expose, over MCP.",
            );

        InitializeResult::new(capabilities)
            .with_server_info(server_info)
            .with_instructions(
                "Every tool here redacts PII before returning document content and \
                 records what it did in an append-only, blake3-hash-chained audit log. \
                 Use `documents_process` to extract and redact whole documents, \
                 `pii_scan`/`pii_redact` for raw text, `pii_reveal` to reverse a \
                 pseudonym token (requires the server to be configured with a \
                 pseudonymisation key), `audit_verify`/`audit_entries` to inspect the \
                 chain, and `compliance_report` for the DPIA/model-card/DORA artefacts.",
            )
    }
}

/// Serialise `value` as the tool's structured result, with a pretty-printed JSON text
/// block alongside it for clients that only render text content.
fn json_tool_result<T: serde::Serialize>(value: &T) -> Result<CallToolResult, rmcp::ErrorData> {
    let json = serde_json::to_value(value).map_err(|error| {
        rmcp::ErrorData::internal_error(format!("failed to serialise tool result: {error}"), None)
    })?;
    let text = serde_json::to_string_pretty(&json).unwrap_or_default();
    let mut result = CallToolResult::success(vec![ContentBlock::text(text)]);
    result.structured_content = Some(json);
    Ok(result)
}

/// Map a [`hacienda::HaciendaError`] onto an MCP tool error.
///
/// Every branch keeps the facade's own message — none of these are the pseudonym-reveal
/// collapse `pii_reveal` handles separately, so there is nothing here that needs hiding.
///
/// The split between `invalid_params` and `internal_error` matters to an MCP client the
/// same way a 4xx/5xx split matters to an HTTP one: `Extraction` (malformed or unsupported
/// input document), `Authz` (the caller lacks a required capability), `PiiDisabled` (the
/// server has no `[pii]` section configured for this call to use), and
/// `RedactionDepthExceeded` (the input archive nests deeper than this node accepts) are all
/// things the caller caused and can act on — a different file, a different key, a different
/// tool, a shallower archive. Everything else (audit/store/review/pseudonym/API-key faults)
/// is this node's own fault; nothing the caller supplied would change the outcome, so it
/// stays `internal_error`.
fn map_hacienda_error(error: hacienda::HaciendaError) -> rmcp::ErrorData {
    match error {
        hacienda::HaciendaError::Extraction(_)
        | hacienda::HaciendaError::Authz(_)
        | hacienda::HaciendaError::PiiDisabled
        | hacienda::HaciendaError::RedactionDepthExceeded { .. } => {
            rmcp::ErrorData::invalid_params(error.to_string(), None)
        }
        other => rmcp::ErrorData::internal_error(other.to_string(), None),
    }
}

/// Map the tamper-detecting [`hacienda_core::audit::AuditError`] variants onto a failure
/// detail string for [`HaciendaMcp::audit_verify`].
///
/// `None` for everything else, which is what keeps I/O and serialisation faults out of the
/// `verified: false` result — those are reported as tool errors instead (see
/// `map_hacienda_error`). Mirrors `hacienda-api/src/handlers/audit.rs`'s
/// `classify_verify_failure`; kept as a plain string here rather than duplicating that
/// handler's `VerifyFailure` wire DTO, since this tool's result is JSON-serialised ad hoc,
/// not schema-checked against an OpenAPI spec.
fn classify_verify_failure(error: &hacienda_core::audit::AuditError) -> Option<String> {
    use hacienda_core::audit::AuditError;
    match error {
        AuditError::ChainIntegrity { .. }
        | AuditError::ConfigMismatch { .. }
        | AuditError::SegmentIntegrity { .. }
        | AuditError::SegmentLink { .. }
        | AuditError::SegmentEntryCount { .. } => Some(error.to_string()),
        AuditError::Io { .. }
        | AuditError::Json(_)
        | AuditError::StoreClosed { .. }
        | AuditError::UnresolvableCursor { .. }
        | AuditError::Backend(_)
        | AuditError::Internal { .. } => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hacienda::{HaciendaConfig, HaciendaFacade};
    use hacienda_core::{pii::PipelineConfig, redaction::RedactionMode};

    /// A value that must never appear in any tool result unredacted. Distinct from
    /// anything else in the codebase, so a match is a disclosure and never a coincidence
    /// — same discipline as `hacienda-api/src/handlers/audit.rs`'s control corpus.
    const CORPUS_EMAIL: &str = "zephyrine.quatrebarbes@corpus-temoin.example";

    fn server_with_pii(mode: RedactionMode) -> HaciendaMcp {
        let mut pii = PipelineConfig::default();
        pii.redaction.mode = mode;
        let config = HaciendaConfig::default().with_pii(pii);
        let facade = Arc::new(HaciendaFacade::new(config).expect("facade"));
        HaciendaMcp::new(facade)
    }

    fn result_text(result: &CallToolResult) -> String {
        result
            .content
            .iter()
            .filter_map(|block| block.as_text().map(|t| t.text.clone()))
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// The whole point of this crate: a tool that redacts must never leak the redacted
    /// value, in either the text content block or the structured content — both are sent
    /// to the MCP client, so both are a disclosure surface.
    #[tokio::test]
    async fn pii_redact_never_returns_the_redacted_value() {
        let server = server_with_pii(RedactionMode::Mask);
        let text = format!("reach me at {CORPUS_EMAIL}");

        let result = server
            .pii_redact(Parameters(TextParams { text }))
            .await
            .expect("tool call succeeds");

        assert!(!result.is_error.unwrap_or(false));
        let rendered = result_text(&result);
        assert!(
            !rendered.contains(CORPUS_EMAIL),
            "pii_redact leaked the corpus value: {rendered}"
        );
        let structured =
            serde_json::to_string(&result.structured_content).expect("structured content");
        assert!(
            !structured.contains(CORPUS_EMAIL),
            "pii_redact's structured_content leaked the corpus value: {structured}"
        );
        assert!(
            rendered.contains("[EMAIL]"),
            "expected a mask token in: {rendered}"
        );
    }

    /// `SpanText::Omit` must survive this transport: `pii_scan` reports categories, never
    /// the entity text, exactly as `POST /v1/pii/scan` does.
    #[tokio::test]
    async fn pii_scan_omits_entity_text() {
        let server = server_with_pii(RedactionMode::Mask);
        let text = format!("reach me at {CORPUS_EMAIL}");

        let result = server
            .pii_scan(Parameters(TextParams { text }))
            .await
            .expect("tool call succeeds");

        let rendered = result_text(&result);
        assert!(
            !rendered.contains(CORPUS_EMAIL),
            "pii_scan leaked the corpus value: {rendered}"
        );
        assert!(
            rendered.contains("\"category\""),
            "expected at least one detected entity in: {rendered}"
        );
    }

    /// A tool call against a server with no `[pii]` section fails with a named error
    /// rather than silently returning the input unredacted.
    #[tokio::test]
    async fn pii_redact_without_pii_configured_is_a_named_error() {
        let facade = Arc::new(HaciendaFacade::new(HaciendaConfig::default()).expect("facade"));
        let server = HaciendaMcp::new(facade);

        let error = server
            .pii_redact(Parameters(TextParams {
                text: CORPUS_EMAIL.to_string(),
            }))
            .await
            .expect_err("must fail without a configured pipeline");
        assert!(error.message.contains("PII"), "{error:?}");
    }

    /// Every tool this crate registers, present and exactly once — a coarse anti-drift
    /// guard against a tool being renamed or dropped without the doc table above being
    /// updated to match.
    #[tokio::test]
    async fn registers_exactly_the_documented_tools() {
        let server = server_with_pii(RedactionMode::Mask);
        let tools = server.tool_router.list_all();
        let mut names: Vec<&str> = tools.iter().map(|t| t.name.as_ref()).collect();
        names.sort_unstable();

        let mut expected = vec![
            "audit_entries",
            "audit_verify",
            "compliance_report",
            "documents_process",
            "get_version",
            "pii_redact",
            "pii_reveal",
            "pii_scan",
        ];
        expected.sort_unstable();
        assert_eq!(names, expected);
    }

    /// `RedactionDepthExceeded` is a client-input-limit violation (an archive nested
    /// deeper than this node accepts), not a server fault — it must classify the same way
    /// `Extraction`/`Authz`/`PiiDisabled` already do (`invalid_params`), not fall through
    /// to `internal_error`. A too-deep archive is exercised end-to-end in
    /// `hacienda-core`'s own
    /// `redact_structured_fields_refuses_to_recurse_past_the_depth_limit`; this test only
    /// pins `map_hacienda_error`'s classification of the resulting error variant.
    #[test]
    fn redaction_depth_exceeded_maps_to_invalid_params_not_internal_error() {
        let error = hacienda::HaciendaError::RedactionDepthExceeded { limit: 64 };
        let mapped = map_hacienda_error(error);
        assert_eq!(
            mapped.code,
            rmcp::ErrorData::invalid_params("", None).code,
            "RedactionDepthExceeded must map to invalid_params, got {mapped:?}"
        );
    }
}
