//! MCP (Model Context Protocol) server for hacienda.
//!
//! Exposes [`hacienda::HaciendaFacade`] to MCP clients (Claude Desktop, agent frameworks)
//! as a set of tools. See `superpowers/specs/2026-08-13-M1-mcp-server-and-cli-sdk-parity-design.md`
//! for the design this crate implements.
//!
//! # Why this is not xberg's own MCP server
//!
//! `xberg` ships its own MCP server behind its `mcp` cargo feature
//! (`crates/xberg/src/mcp/`): nine raw tools including `extract`, `extract_batch`,
//! `detect_mime_type`, `list_formats`, version, and cache-management operations, all
//! returning **unredacted** document content — there is no
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
//! [`HaciendaMcp`] calls its facade methods with `Caller::Trusted`, so the process
//! boundary is not sufficient to treat an agent as a human reviewer. In particular, the
//! MCP tool inventory deliberately excludes pseudonym rehydration: `pii_reveal` would
//! return plaintext into Claude/Codex context. Human rehydration belongs in the
//! authenticated and audited Tauri viewer. This crate implements **stdio** (M1a): a local
//! MCP client launches `hacienda mcp serve` as a child process. An HTTP/SSE transport
//! (M1b) is not implemented here.
//!
//! # Tool inventory
//!
//! Thirteen tools, each a thin wrapper over one [`hacienda::HaciendaFacade`] method already
//! exercised by `hacienda-api`'s handlers or `hacienda-cli`'s subcommands:
//!
//! | Tool | Facade method | Mirrors |
//! | --- | --- | --- |
//! | `documents_process` | `process_batch_with_auth` | `POST /v1/documents`, `hacienda extract` |
//! | `pii_scan` | `scan_text_with_auth` | `POST /v1/pii/scan` |
//! | `pii_redact` | `redact_text_with_auth` | `POST /v1/pii/redact` |
//! | `audit_verify` | `verify_audit_with_auth` | `GET /v1/audit/verify` |
//! | `audit_entries` | `audit_history_with_auth` | `GET /v1/audit/entries` |
//! | `compliance_report` | `compliance_report_with_auth` | `GET /v1/compliance/report` |
//! | `get_capabilities` | (static) | `GET /capabilities`, `hacienda capabilities` |
//! | `list_formats` | (Xberg) | `GET /v1/formats`, `hacienda formats` |
//! | `detect_mime_type` | (Xberg) | `POST /v1/mime/detect`, `hacienda mime` |
//! | `cache_stats` | (Xberg) | `GET /v1/cache/stats`, `hacienda cache stats` |
//! | `cache_clear` | (Xberg) | `POST /v1/cache/clear`, `hacienda cache clear` |
//! | `cache_warm` | (Xberg) | `POST /v1/cache/warm`, `hacienda cache warm` |
//! | `get_version` | (static) | `GET /version` |

mod params;

use std::sync::Arc;

use hacienda::HaciendaFacade;
use hacienda_core::{auth::Caller, facade::SpanText};
use params::{
    AuditEntriesParams, CacheConfirmParams, CacheWarmParams, DocumentsProcessParams, EmptyParams,
    MimeDetectParams, TextParams,
};
use rmcp::{
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::*,
    tool, tool_handler, tool_router,
    transport::{async_rw::AsyncRwTransport, stdio},
    RoleServer, ServerHandler, ServiceExt,
};
use xberg::ExtractInput;

/// Page size used when a tool call does not ask for one, mirroring
/// `hacienda-api/src/handlers/audit.rs`'s `DEFAULT_PAGE_LIMIT`.
const DEFAULT_PAGE_LIMIT: usize = 100;
/// Largest page this tool will serve, mirroring that module's `MAX_PAGE_LIMIT`.
const MAX_PAGE_LIMIT: usize = 1000;
/// Maximum size of one JSON-RPC request line accepted over MCP stdio.
///
/// This leaves room for the 8 MiB MIME payload after base64 encoding and its
/// JSON-RPC envelope, while bounding the transport's per-message allocation.
const MAX_MCP_REQUEST_BYTES: usize = 16 * 1024 * 1024;

/// The hacienda MCP server. Holds the facade every tool call routes through.
#[derive(Clone)]
pub struct HaciendaMcp {
    facade: Arc<HaciendaFacade>,
    // Used by the `#[tool_handler(router = self.tool_router)]` dispatch methods.
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
        let (reader, writer) = stdio();
        let transport = AsyncRwTransport::<RoleServer, _, _>::new(reader, writer)
            .with_max_line_length(MAX_MCP_REQUEST_BYTES);
        let service = self.serve(transport).await?;
        service.waiting().await?;
        Ok(())
    }

    /// Extract and redact one or more documents.
    #[tool(
        description = "Extract text from documents (string local paths or file:// URIs, or \
            structured `{kind: bytes|uri, ...}` inputs with MIME/filename hints) and redact \
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
        //
        // A conversion failure names the offending input's index: a batch with one lying
        // `mime_type` must tell the caller which document was refused, not just that
        // some document was. The index is attached by this loop, so the error message
        // never needs to carry — and never carries — any document content.
        let inputs: Vec<ExtractInput> = params
            .inputs
            .into_iter()
            .enumerate()
            .map(|(index, input)| {
                input.into_extract_input().map_err(|error| {
                    rmcp::ErrorData::invalid_params(format!("inputs[{index}]: {error}"), None)
                })
            })
            .collect::<Result<_, _>>()?;
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
                ));
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

    /// Report the compiled and runtime capability state shared with REST and CLI.
    #[tool(
        description = "Report safe build and runtime capability state. This never returns \
            document content, secrets, model paths, or authorization grants.",
        annotations(
            title = "Get Capabilities",
            read_only_hint = true,
            idempotent_hint = true
        )
    )]
    fn get_capabilities(
        &self,
        Parameters(_): Parameters<EmptyParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let report = hacienda_core::capabilities::report(
            env!("CARGO_PKG_VERSION"),
            false,
            hacienda_core::capabilities::cached_onnx_runtime_status(),
            false,
        );
        json_tool_result(&report)
    }

    #[tool(
        description = "List Xberg document formats enabled in Hacienda.",
        annotations(title = "List Formats", read_only_hint = true, idempotent_hint = true)
    )]
    fn list_formats(
        &self,
        Parameters(_): Parameters<EmptyParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        json_tool_result(&serde_json::json!({
            "formats": hacienda_core::xberg_surface::supported_formats()
        }))
    }

    #[tool(
        description = "Detect MIME type from inline base64 bytes without reading a path or URL.",
        annotations(
            title = "Detect MIME Type",
            read_only_hint = true,
            idempotent_hint = true
        )
    )]
    fn detect_mime_type(
        &self,
        Parameters(params): Parameters<MimeDetectParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        if params.content_base64.len() > hacienda_core::xberg_surface::MAX_MIME_INPUT_BASE64_BYTES {
            return Err(rmcp::ErrorData::invalid_params(
                "content_base64 exceeds the 8 MiB limit",
                None,
            ));
        }
        let bytes = data_encoding::BASE64
            .decode(params.content_base64.as_bytes())
            .map_err(|_| rmcp::ErrorData::invalid_params("content_base64 is invalid", None))?;
        let mime_type =
            hacienda_core::xberg_surface::detect_mime_type(&bytes, params.filename.as_deref())
                .map_err(|_| {
                    rmcp::ErrorData::invalid_params("unsupported document content", None)
                })?;
        json_tool_result(&serde_json::json!({ "mime_type": mime_type }))
    }

    #[tool(
        description = "Return sanitized Xberg cache statistics without filesystem paths.",
        annotations(title = "Cache Stats", read_only_hint = true, idempotent_hint = true)
    )]
    async fn cache_stats(
        &self,
        Parameters(_): Parameters<EmptyParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let stats = tokio::task::spawn_blocking(hacienda_core::xberg_surface::cache_stats)
            .await
            .map_err(|_| rmcp::ErrorData::internal_error("cache stats failed", None))?
            .map_err(|_| rmcp::ErrorData::internal_error("cache stats failed", None))?;
        json_tool_result(&stats)
    }

    #[tool(
        description = "Clear Xberg-owned cache files and resident engines. Requires confirm=true.",
        annotations(title = "Clear Cache", read_only_hint = false, destructive_hint = true)
    )]
    async fn cache_clear(
        &self,
        Parameters(params): Parameters<CacheConfirmParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        if !params.confirm {
            return Err(rmcp::ErrorData::invalid_params(
                "confirm must be true",
                None,
            ));
        }
        let result = tokio::task::spawn_blocking(hacienda_core::xberg_surface::clear_cache)
            .await
            .map_err(|_| rmcp::ErrorData::internal_error("cache clear failed", None))?
            .map_err(|_| rmcp::ErrorData::internal_error("cache clear failed", None))?;
        json_tool_result(&result)
    }

    #[tool(
        description = "Warm one compiled Xberg embedding preset. Requires confirm=true.",
        annotations(title = "Warm Cache", read_only_hint = false, open_world_hint = true)
    )]
    async fn cache_warm(
        &self,
        Parameters(params): Parameters<CacheWarmParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        if !params.confirm {
            return Err(rmcp::ErrorData::invalid_params(
                "confirm must be true",
                None,
            ));
        }
        let result = tokio::time::timeout(
            std::time::Duration::from_secs(30 * 60),
            hacienda_core::xberg_surface::warm_embedding_preset(&params.preset),
        )
        .await
        .map_err(|_| rmcp::ErrorData::internal_error("cache warm timed out", None))?
        .map_err(|error| match error {
            hacienda_core::xberg_surface::XbergSurfaceError::WarmUnavailable => {
                rmcp::ErrorData::invalid_params("embedding preset unavailable", None)
            }
            hacienda_core::xberg_surface::XbergSurfaceError::Warm(_) => {
                rmcp::ErrorData::internal_error("embedding preset warm failed", None)
            }
            hacienda_core::xberg_surface::XbergSurfaceError::Mime(_)
            | hacienda_core::xberg_surface::XbergSurfaceError::Cache(_)
            | hacienda_core::xberg_surface::XbergSurfaceError::UnsafeCacheRoot
            | hacienda_core::xberg_surface::XbergSurfaceError::CacheBusy => {
                rmcp::ErrorData::internal_error("embedding preset warm failed", None)
            }
            hacienda_core::xberg_surface::XbergSurfaceError::MimeInputTooLarge => {
                rmcp::ErrorData::invalid_params("input too large", None)
            }
        })?;
        json_tool_result(&result)
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

#[tool_handler(router = self.tool_router)]
impl ServerHandler for HaciendaMcp {
    fn get_info(&self) -> ServerConfig {
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
                "Tools redact PII before returning document content and record what they \
                 do in an append-only, blake3-hash-chained audit log. Pseudonym \
                 rehydration is intentionally unavailable to agent clients; use the \
                 authenticated, audited Tauri viewer for human review. \
                 Use `documents_process` to extract and redact whole documents, \
                 `pii_scan`/`pii_redact` for raw text, `audit_verify`/`audit_entries` \
                 to inspect the chain, and `compliance_report` for the \
                 DPIA/model-card/DORA artefacts.",
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
/// Every branch keeps the facade's own message; sensitive pseudonym reveal is not exposed
/// through this agent-facing server.
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
    use params::{DocumentInput, StructuredDocumentInput};

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
            "cache_clear",
            "cache_stats",
            "cache_warm",
            "detect_mime_type",
            "get_capabilities",
            "get_version",
            "list_formats",
            "pii_redact",
            "pii_scan",
        ];
        expected.sort_unstable();
        assert_eq!(names, expected);
    }

    /// Issue #147: the agent-facing manifest must never expose a reveal/rehydration tool —
    /// no `RevealTicket` can be minted or resolved through `Caller::Trusted` here. This is
    /// the manifest-level guard; the facade refuses `Caller::Trusted` ticket minting too
    /// (`HaciendaFacade::mint_reveal_ticket_with_auth`).
    #[tokio::test]
    async fn manifest_contains_no_reveal_or_rehydration_tool() {
        let server = server_with_pii(RedactionMode::Mask);
        let tools = server.tool_router.list_all();
        for tool in &tools {
            let name = tool.name.as_ref();
            assert!(
                !name.contains("reveal") && !name.contains("rehydrat"),
                "agent-facing manifest must not expose '{name}'"
            );
        }
    }

    #[test]
    fn capabilities_tool_uses_the_shared_report_shape() {
        let server = server_with_pii(RedactionMode::Mask);
        let result = server
            .get_capabilities(Parameters(EmptyParams {}))
            .expect("capability probe must serialize");
        let content = result
            .structured_content
            .expect("capability probe must include structured content");

        assert_eq!(content["embedding_mode"], "presets_only");
        assert_eq!(content["capabilities"][0]["name"], "redaction");
        assert_eq!(content["onnx_runtime"]["compiled"], false);
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

    #[test]
    fn structured_bytes_input_preserves_filename_and_mime_type() {
        let input: DocumentInput = serde_json::from_value(serde_json::json!({
            "kind": "bytes",
            "bytes": [72, 105],
            "mime_type": "text/plain",
            "filename": "note.txt"
        }))
        .expect("structured bytes input");

        let input = input.into_extract_input().expect("valid bytes input");
        assert_eq!(input.bytes, Some(vec![72, 105]));
        assert_eq!(input.mime_type.as_deref(), Some("text/plain"));
        assert_eq!(input.filename.as_deref(), Some("note.txt"));
        assert!(input.uri.is_none());
    }

    #[test]
    fn structured_input_rejects_per_call_extraction_config() {
        let error = serde_json::from_value::<DocumentInput>(serde_json::json!({
            "kind": "uri",
            "uri": "file:///tmp/document.txt",
            "config": {"extract_tables": false}
        }))
        .expect_err("per-call config must not be accepted");

        assert!(error.to_string().contains("unknown field"), "{error}");
    }

    /// Bytes input (`kind: "bytes"`, xberg's structured form) runs through the same
    /// `process_batch_with_auth` seam as the CLI — the corpus value must not surface in
    /// either the text content block or `structured_content`, both of which reach the
    /// MCP client.
    #[tokio::test]
    async fn documents_process_redacts_structured_bytes_input() {
        let server = server_with_pii(RedactionMode::Mask);
        let result = server
            .documents_process(Parameters(DocumentsProcessParams {
                inputs: vec![DocumentInput::Structured(StructuredDocumentInput::Bytes {
                    bytes: format!("reach me at {CORPUS_EMAIL}").into_bytes(),
                    mime_type: "text/plain".to_string(),
                    filename: Some("note.txt".to_string()),
                })],
            }))
            .await
            .expect("tool call succeeds");

        assert!(!result.is_error.unwrap_or(false));
        let rendered = result_text(&result);
        assert!(
            !rendered.contains(CORPUS_EMAIL),
            "documents_process leaked the corpus value: {rendered}"
        );
        let structured =
            serde_json::to_string(&result.structured_content).expect("structured content");
        assert!(
            !structured.contains(CORPUS_EMAIL),
            "documents_process's structured_content leaked the corpus value: {structured}"
        );
        assert!(
            rendered.contains("[EMAIL]"),
            "expected a mask token in: {rendered}"
        );
    }

    /// The legacy string form is still a URI shorthand: a local file written by the test
    /// is extracted and redacted through the same facade seam.
    #[tokio::test]
    async fn documents_process_redacts_uri_shorthand_input() {
        use std::io::Write as _;

        let server = server_with_pii(RedactionMode::Mask);
        let mut file = tempfile::NamedTempFile::new().expect("temp file");
        write!(file, "reach me at {CORPUS_EMAIL}").expect("writes fixture");

        let result = server
            .documents_process(Parameters(DocumentsProcessParams {
                inputs: vec![DocumentInput::Uri(
                    file.path().to_string_lossy().into_owned(),
                )],
            }))
            .await
            .expect("tool call succeeds");

        assert!(!result.is_error.unwrap_or(false));
        let rendered = result_text(&result);
        assert!(
            !rendered.contains(CORPUS_EMAIL),
            "documents_process leaked the corpus value: {rendered}"
        );
        assert!(
            rendered.contains("[EMAIL]"),
            "expected a mask token in: {rendered}"
        );
    }

    /// CodeRabbit follow-up (#157): a refused input must name its index, so a caller
    /// can tell *which* document in a batch was rejected — the conversion errors attach
    /// the field, the cause, and a suggestion, and this loop attaches the position.
    #[tokio::test]
    async fn documents_process_names_the_refused_input_index() {
        let server = server_with_pii(RedactionMode::Mask);
        let error = server
            .documents_process(Parameters(DocumentsProcessParams {
                inputs: vec![
                    DocumentInput::Uri("/nonexistent/first.txt".to_string()),
                    DocumentInput::Structured(StructuredDocumentInput::Bytes {
                        bytes: b"%PDF-1.7\n".to_vec(),
                        mime_type: "text/plain".to_string(),
                        filename: None,
                    }),
                ],
            }))
            .await
            .expect_err("the lying MIME hint must refuse the batch");

        let message = error.message;
        assert!(message.contains("inputs[1]"), "{message}");
        assert!(
            message.contains("suggestion:"),
            "conversion errors must carry repair guidance: {message}"
        );
        assert!(
            !message.contains("%PDF"),
            "the error must not echo document content: {message}"
        );
    }
}
