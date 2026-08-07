//! Handlers for the five audit endpoints.
//!
//! | Route | Capability | Answers |
//! | --- | --- | --- |
//! | `GET /v1/audit/entries` | `audit:read` | one page of this node's history |
//! | `GET /v1/audit/verify` | `audit:read` | are the chain and the seals intact |
//! | `GET /v1/audit/seals` | `audit:read` | the segment seals, oldest first |
//! | `GET /v1/audit/export` | `audit:export` | the whole history as bytes |
//! | `GET /v1/audit/tip` | `documents:process` | the current chain head |
//!
//! # `tip` is deliberately not behind `audit:read`
//!
//! The tip is an opaque blake3 digest that reveals nothing about the entries behind it.
//! Gating it would stop a caller holding `documents:process` from obtaining the chain
//! evidence for **its own** result — every content-bearing response already carries the
//! tip for exactly that purpose, so gating the dedicated endpoint would be inconsistent as
//! well as unhelpful. `HaciendaFacade::audit_tip` documents the same decision.
//!
//! # This node, not this deployment
//!
//! Segments are per-writer and there is no total order between writers, so a server can
//! only speak for its own chain. Every response here carries `"scope": "this_node"`, and
//! the export carries `x-hacienda-audit-scope: this-node`. Saying it only in prose would
//! repeat, at the HTTP layer, the mistake `AuditStore::history` exists to close.
//!
//! # Sealed-segment reads run inline, and that is a decision
//!
//! `history()`, `verify()` and `export_store()` read sealed segment files with blocking
//! `std::fs` calls, so a handler here occupies a runtime worker for the duration of that
//! I/O. That is not new — `verify()` has always behaved this way — but this is the first
//! time it is reachable over HTTP, so it is decided rather than discovered under load:
//!
//! **These handlers do not wrap the facade calls in `spawn_blocking`.** Three reasons.
//!
//! 1. *The blocking is in the store, and so is the fix.* `FileAuditStore` is what touches
//!    the disk; moving its reads onto a blocking pool there fixes every caller at once —
//!    the CLI, the facade, a future gRPC surface. Wrapping at one of several call sites
//!    fixes one of them and leaves the same trap set for the next.
//! 2. *It cannot be done cleanly from here anyway.* `AuditStore`'s methods are `async fn`
//!    on `&self` taking `Option<&AuditCursor>`, and the facade does not hand out its
//!    `Arc<dyn AuditStore>`. Wrapping from a handler would mean a `Handle::block_on`
//!    inside `spawn_blocking` — trading a blocked worker for a blocked blocking-pool
//!    thread plus a re-entrant runtime call, which is worse on both counts.
//! 3. *The traffic shape does not warrant it.* These are operator and auditor requests,
//!    not the per-document hot path, and the worker pool is sized to the core count.
//!
//! What is done instead is to bound the exposure where it is cheap: `?limit` is capped at
//! [`MAX_PAGE_LIMIT`], so one `/v1/audit/entries` call cannot be made to read the whole
//! history. `/v1/audit/export` is inherently unbounded — it reads everything by
//! definition — and is therefore the first thing to move if this ever needs revisiting.

use axum::{
    extract::State,
    http::{header, request::Parts, HeaderMap, HeaderName, HeaderValue},
    Json,
};
use hacienda_core::{
    audit::{AuditCursor, AuditError, ExportFormat},
    error::HaciendaError,
};
use serde::Deserialize;

use crate::{
    dto::{
        AuditScope, AuditTipResponse, NodeAuditEntryDto, NodeAuditPage, NodeSealsResponse,
        SegmentSealDto, VerifyFailure, VerifyFailureKind, VerifyResponse,
    },
    error::ApiError,
    extract::Query,
    handlers::{caller_from_arc, extract_auth_context},
    state::ApiState,
};

/// Page size used when the caller does not ask for one.
pub const DEFAULT_PAGE_LIMIT: usize = 100;

/// Largest page this endpoint will serve, whatever `?limit` says.
pub const MAX_PAGE_LIMIT: usize = 1000;

// ── Query types ───────────────────────────────────────────────────────────────

/// `GET /v1/audit/entries` query.
///
/// # Why there are no filters here
///
/// The spec sketches `from`, `to`, `category`, `action` and `principal`. None are
/// implemented, and `deny_unknown_fields` makes asking for one a 400 rather than a
/// silently unfiltered answer.
///
/// Filtering cannot be bolted on above the cursor without breaking the paging contract:
/// the client's stop condition is "page until you get an empty page", and post-filtering a
/// page would produce empty pages while unread entries remain — a paginating auditor would
/// stop early and never learn they had. Filtering belongs in `AuditStore::history`, where
/// it can consume entries until the page is full. Until it lives there, refusing the
/// parameter is the only answer that cannot mislead.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EntriesQuery {
    /// Entries per page. Omitted → [`DEFAULT_PAGE_LIMIT`]. Above [`MAX_PAGE_LIMIT`] →
    /// clamped. Zero → 400; see [`resolve_limit`].
    limit: Option<usize>,
    /// An opaque cursor previously returned as `next_cursor`.
    cursor: Option<String>,
}

/// `GET /v1/audit/export` query.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExportQuery {
    /// `json` (the default), `jsonl`, or `csv`.
    format: Option<String>,
}

/// A query type for the endpoints that take no parameters.
///
/// Not `()`: an endpoint with no parameters should reject the ones it does not have, for
/// the same reason [`EntriesQuery`] does. A caller passing `?limit=10` to
/// `/v1/audit/seals` has a wrong belief about the endpoint, and 400 corrects it.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NoQuery {}

// ── GET /v1/audit/entries ─────────────────────────────────────────────────────

/// `GET /v1/audit/entries?limit=&cursor=`
///
/// One page of this node's history, oldest first, in chain order. Ordering is not
/// negotiable and there is no sort parameter: the chain is a sequence, and its order
/// carries meaning that a column sort would destroy.
///
/// Page until you receive an **empty page** — not until `next_cursor` is `null`. See
/// [`NodeAuditPage`].
#[utoipa::path(
    get,
    path = "/v1/audit/entries",
    tag = "audit",
    operation_id = "getAuditEntries",
    description = "Requires audit:read",
    security(("bearerAuth" = [])),
    params(
        ("limit" = Option<usize>, Query, description = "Entries per page (default 100, capped at 1000); 0 is refused"),
        ("cursor" = Option<String>, Query, description = "Opaque cursor from a previous page's next_cursor")
    ),
    responses((status = 200, description = "One page of this node's audit history, oldest first", body = NodeAuditPage))
)]
pub async fn audit_entries(
    State(state): State<ApiState>,
    parts: Parts,
    Query(query): Query<EntriesQuery>,
) -> Result<Json<NodeAuditPage>, ApiError> {
    let ctx = extract_auth_context(&parts);
    let caller = caller_from_arc(&ctx);

    let limit = resolve_limit(query.limit)?;
    let cursor = match query.cursor.as_deref() {
        None => None,
        // The parse error quotes the cursor the client sent; that string is
        // attacker-controlled, so it is dropped and a constant sentence takes its place.
        Some(raw) => Some(raw.parse::<AuditCursor>().map_err(|_| malformed_cursor())?),
    };

    let page = state
        .facade
        .audit_history_with_auth(caller, cursor.as_ref(), limit)
        .await
        .map_err(read_error)?
        .ok_or_else(ApiError::audit_not_configured)?;

    Ok(Json(NodeAuditPage {
        scope: AuditScope::ThisNode,
        entries: page.entries.into_iter().map(NodeAuditEntryDto::from).collect(),
        next_cursor: page.next.map(|cursor| cursor.to_string()),
    }))
}

/// Turn `?limit` into a page size.
///
/// # Zero is refused, an over-large value is clamped
///
/// The asymmetry is deliberate. `AuditStore::history` answers `limit == 0` with an empty
/// page and no cursor — byte-identical to "you are caught up" — so a client whose paging
/// variable was never initialised would loop forever, or worse, conclude the history was
/// empty. There is no interpretation of "give me zero entries" that a caller could have
/// meant, so it is a 400 that names the parameter.
///
/// An over-large limit does have an unambiguous safe reading: serve fewer and hand back a
/// cursor. The caller's page-until-empty loop is unaffected and simply runs another
/// iteration, so clamping costs it nothing and refusing would cost it a round trip and an
/// error path for no gain.
fn resolve_limit(requested: Option<usize>) -> Result<usize, ApiError> {
    match requested {
        None => Ok(DEFAULT_PAGE_LIMIT),
        Some(0) => Err(ApiError::invalid_request(format!(
            "`limit` must be at least 1 (default {DEFAULT_PAGE_LIMIT}, maximum \
             {MAX_PAGE_LIMIT}). A limit of 0 returns an empty page, which is \
             indistinguishable from having reached the end of the history."
        ))),
        Some(n) => Ok(n.min(MAX_PAGE_LIMIT)),
    }
}

// ── GET /v1/audit/verify ──────────────────────────────────────────────────────

/// `GET /v1/audit/verify`
///
/// Runs the three integrity checks — the seal chain, each sealed segment's entry chain
/// against its seal, and the open segment — and reports the verdict.
///
/// **A broken chain is a 200 with `verified: false`,** naming the offending entry or seal.
/// It is an answer, not a server failure: the caller asked whether the chain was intact and
/// it is not. A 500 would say the server could not answer, would bury the identity of the
/// bad record inside the generic error envelope, and would be indistinguishable from a
/// crashed handler — leaving the one client who most needs the detail with none of it.
///
/// A read failure — I/O, deserialisation — is still a 500, because it is genuinely not a
/// verdict on the chain. See [`VerifyFailureKind`].
#[utoipa::path(
    get,
    path = "/v1/audit/verify",
    tag = "audit",
    // Kept as "verifyAudit", not renamed: this operationId is what generated SDKs derive
    // their function name from (sdks/python/src/hacienda_sdk/client.py imports
    // `verify_audit` from the generated module by exactly this name), and swapping the
    // route's handler here is not a wire-contract rename.
    operation_id = "verifyAudit",
    description = "Requires audit:read",
    security(("bearerAuth" = [])),
    responses((status = 200, description = "Verification verdict for this node's chain", body = VerifyResponse))
)]
pub async fn audit_verify(
    State(state): State<ApiState>,
    parts: Parts,
    Query(_query): Query<NoQuery>,
) -> Result<Json<VerifyResponse>, ApiError> {
    let ctx = extract_auth_context(&parts);
    let caller = caller_from_arc(&ctx);

    // The capability check happens inside the facade call, so it must run before the
    // "is auditing configured" question is answered — otherwise an unauthorised caller
    // learns the server's configuration from the status code.
    let verdict = state.facade.verify_audit_with_auth(caller).await;

    if !state.facade.audit_enabled() {
        // `verify_audit_with_auth` returns `Ok(())` for a facade with no store, which is
        // honest as "nothing failed" and a lie as "the chain is intact". Refuse to report
        // a verified chain where there is no chain.
        return Err(ApiError::audit_not_configured());
    }

    match verdict {
        Ok(()) => Ok(Json(VerifyResponse {
            scope: AuditScope::ThisNode,
            verified: true,
            failure: None,
        })),
        Err(HaciendaError::Audit(error)) => match classify_verify_failure(&error) {
            Some(failure) => Ok(Json(VerifyResponse {
                scope: AuditScope::ThisNode,
                verified: false,
                failure: Some(failure),
            })),
            // Not a tamper verdict — an I/O or serialisation failure. Propagate it as an
            // error so it lands in the host's error rate rather than being reported to an
            // auditor as tampering.
            None => Err(ApiError::from(HaciendaError::Audit(error))),
        },
        Err(other) => Err(ApiError::from(other)),
    }
}

/// Map the tamper-detecting [`AuditError`] variants onto a wire failure.
///
/// `None` for everything else, which is what keeps I/O and serialisation faults out of the
/// 200 path. The rendered `detail` is taken only from the matched variants, whose `Display`
/// carries segment ids, hashes and counts — never a filesystem path, never document
/// content.
fn classify_verify_failure(error: &AuditError) -> Option<VerifyFailure> {
    let detail = error.to_string();
    match error {
        AuditError::ChainIntegrity {
            index,
            expected,
            actual,
        } => Some(VerifyFailure {
            kind: VerifyFailureKind::ChainIntegrity,
            detail,
            // Core's entry-chain check does not carry the segment through; see
            // `VerifyFailure`.
            segment_id: None,
            entry_index: Some(*index),
            expected_hash: Some(expected.clone()),
            actual_hash: Some(actual.clone()),
        }),
        AuditError::SegmentIntegrity {
            segment_id,
            expected,
            actual,
        } => Some(VerifyFailure {
            kind: VerifyFailureKind::SegmentIntegrity,
            detail,
            segment_id: Some(segment_id.clone()),
            entry_index: None,
            expected_hash: Some(expected.clone()),
            actual_hash: Some(actual.clone()),
        }),
        AuditError::SegmentLink {
            segment_id,
            expected,
            actual,
        } => Some(VerifyFailure {
            kind: VerifyFailureKind::SegmentLink,
            detail,
            segment_id: Some(segment_id.clone()),
            entry_index: None,
            expected_hash: Some(expected.clone()),
            actual_hash: Some(actual.clone()),
        }),
        AuditError::SegmentEntryCount { segment_id, .. } => Some(VerifyFailure {
            kind: VerifyFailureKind::SegmentEntryCount,
            detail,
            segment_id: Some(segment_id.clone()),
            entry_index: None,
            expected_hash: None,
            actual_hash: None,
        }),
        AuditError::ConfigMismatch { .. } => Some(VerifyFailure {
            kind: VerifyFailureKind::ConfigMismatch,
            detail,
            segment_id: None,
            entry_index: None,
            expected_hash: None,
            actual_hash: None,
        }),
        _ => None,
    }
}

// ── GET /v1/audit/seals ───────────────────────────────────────────────────────

/// `GET /v1/audit/seals`
///
/// Every seal this node holds, oldest first, each with its `entry_count` and `seal_hash`.
/// The seals are what make the deletion of a whole segment detectable, and they are what a
/// client passes to `verify_seal_chain` to check that offline.
#[utoipa::path(
    get,
    path = "/v1/audit/seals",
    tag = "audit",
    operation_id = "getAuditSeals",
    description = "Requires audit:read",
    security(("bearerAuth" = [])),
    responses((status = 200, description = "Every segment seal this node holds, oldest first", body = NodeSealsResponse))
)]
pub async fn audit_seals(
    State(state): State<ApiState>,
    parts: Parts,
    Query(_query): Query<NoQuery>,
) -> Result<Json<NodeSealsResponse>, ApiError> {
    let ctx = extract_auth_context(&parts);
    let caller = caller_from_arc(&ctx);

    let seals = state
        .facade
        .audit_seals_with_auth(caller)
        .await
        .map_err(read_error)?
        .ok_or_else(ApiError::audit_not_configured)?;

    Ok(Json(NodeSealsResponse {
        scope: AuditScope::ThisNode,
        seals: seals.into_iter().map(SegmentSealDto::from).collect(),
    }))
}

// ── GET /v1/audit/export ──────────────────────────────────────────────────────

/// `GET /v1/audit/export?format=json|jsonl|csv`
///
/// # The two artefacts, and why the response shouts about which one it is
///
/// `json` and `jsonl` return every entry across every segment, in chain order, as a **flat,
/// chain-ordered export**: consecutive entries' `chain_hash`es can be recomputed and checked
/// offline against each other, which is what makes it verifiable. `csv` returns the same
/// entries as a **tabular extract**, which cannot be verified at all — a flat table's rows
/// have no reliable adjacency guarantee once opened in different tools, so no row's
/// `chain_hash` can be trusted to check against its neighbour. Adding columns would not fix
/// that; it is structural.
///
/// This is *not* the segment-grouped, seal-embedding evidence envelope the format was
/// originally speced to produce (seals are a separate call, `GET /v1/audit/seals`) — that
/// richer shape is unimplemented; `hacienda_core::audit::export::export_json` only
/// serialises `chain.entries()`. Track building the grouped envelope separately rather than
/// reading this doc comment as already describing it.
///
/// Someone handing a regulator a CSV in the belief that it is probative is the failure this
/// whole spec exists to prevent, so the distinction the code *does* make (json/jsonl
/// verifiable, csv not) is carried three ways:
///
/// - `x-hacienda-audit-evidence: envelope | none` — machine-readable, for a client that
///   automates the hand-off.
/// - the `content-disposition` filename — `hacienda-audit-evidence-envelope.json` versus
///   `hacienda-audit-extract-NOT-EVIDENCE.csv`. This is the one that matters most: headers
///   are gone the moment the file is saved, and the mistake happens later, when somebody
///   attaches a file out of their downloads folder to an email.
/// - the content type.
///
/// # Why not a field inside the payload
///
/// For CSV there is nowhere to put one: a comment row is not RFC 4180 and breaks every
/// spreadsheet importer, and an extra column would be attached to each row rather than to
/// the file. For JSON the exported bytes are the thing an offline verifier parses, and
/// this handler must not alter them — the whole guarantee is that what the server sent is
/// what the verifier checks. So the marker lives in the envelope of the *response*, not in
/// the evidence.
///
/// `format` defaults to `json`: an unspecified request must never yield the
/// non-probative artefact.
#[utoipa::path(
    get,
    path = "/v1/audit/export",
    tag = "audit",
    operation_id = "exportAudit",
    description = "Requires audit:export",
    security(("bearerAuth" = [])),
    params(
        ("format" = Option<String>, Query, description = "json (default) or jsonl — flat, chain-ordered export, verifiable offline via consecutive chain_hash recomputation; csv — tabular extract, not verifiable")
    ),
    responses(
        (status = 200, description = "The whole history, oldest first: a JSON array of NodeAuditEntryDto (format=json, the default), one such object per line (format=jsonl), or a CSV table with an id,timestamp,category,... header row (format=csv). See `x-hacienda-audit-evidence` and this operation's own doc comment for which formats are offline-verifiable.", content(
            (Vec<NodeAuditEntryDto> = "application/json"),
            (String = "application/x-ndjson"),
            (String = "text/csv")
        ))
    )
)]
pub async fn audit_export(
    State(state): State<ApiState>,
    parts: Parts,
    Query(query): Query<ExportQuery>,
) -> Result<(HeaderMap, Vec<u8>), ApiError> {
    let ctx = extract_auth_context(&parts);
    let caller = caller_from_arc(&ctx);

    let format = parse_format(query.format.as_deref())?;

    let bytes = state
        .facade
        .audit_export_with_auth(caller, format)
        .await
        .map_err(read_error)?
        .ok_or_else(ApiError::audit_not_configured)?;

    Ok((export_headers(format), bytes))
}

/// Parse `?format`, defaulting to the verifiable artefact.
fn parse_format(raw: Option<&str>) -> Result<ExportFormat, ApiError> {
    match raw {
        None | Some("json") => Ok(ExportFormat::Json),
        Some("jsonl") => Ok(ExportFormat::JsonLines),
        Some("csv") => Ok(ExportFormat::Csv),
        // The rejected value is not echoed: it is client-supplied and would land in the
        // response body and the host's logs.
        Some(_) => Err(ApiError::invalid_request(
            "`format` must be one of `json`, `jsonl` (evidence envelopes, verifiable \
             offline) or `csv` (a tabular extract, not verifiable). Defaults to `json`.",
        )),
    }
}

/// The headers that tell a client — and the filesystem it saves to — which artefact this is.
fn export_headers(format: ExportFormat) -> HeaderMap {
    let (content_type, filename, evidence) = match format {
        ExportFormat::Json => (
            "application/json",
            "attachment; filename=\"hacienda-audit-evidence-envelope.json\"",
            "envelope",
        ),
        ExportFormat::JsonLines => (
            "application/x-ndjson",
            "attachment; filename=\"hacienda-audit-evidence-envelope.jsonl\"",
            "envelope",
        ),
        ExportFormat::Csv => (
            "text/csv; charset=utf-8",
            "attachment; filename=\"hacienda-audit-extract-NOT-EVIDENCE.csv\"",
            "none",
        ),
    };

    let mut headers = HeaderMap::new();
    headers.insert(header::CONTENT_TYPE, HeaderValue::from_static(content_type));
    headers.insert(
        header::CONTENT_DISPOSITION,
        HeaderValue::from_static(filename),
    );
    headers.insert(
        HeaderName::from_static("x-hacienda-audit-evidence"),
        HeaderValue::from_static(evidence),
    );
    headers.insert(
        HeaderName::from_static("x-hacienda-audit-scope"),
        HeaderValue::from_static("this-node"),
    );
    headers
}

// ── GET /v1/audit/tip ─────────────────────────────────────────────────────────

/// `GET /v1/audit/tip`
///
/// The current head of this node's chain. Guarded by `documents:process` rather than
/// `audit:read` — see the module docs.
#[utoipa::path(
    get,
    path = "/v1/audit/tip",
    tag = "audit",
    operation_id = "getAuditTip",
    description = "Requires documents:process",
    security(("bearerAuth" = [])),
    responses((status = 200, description = "The current head of this node's chain", body = AuditTipResponse))
)]
pub async fn audit_tip(
    State(state): State<ApiState>,
    Query(_query): Query<NoQuery>,
) -> Result<Json<AuditTipResponse>, ApiError> {
    // No caller is derived here on purpose: `audit_tip` is not capability-guarded at the
    // facade, and the route's own `documents:process` requirement has already been
    // enforced by the middleware.
    let tip = state
        .facade
        .audit_tip()
        .await
        .map_err(ApiError::from)?
        .ok_or_else(ApiError::audit_not_configured)?;

    Ok(Json(AuditTipResponse {
        scope: AuditScope::ThisNode,
        tip,
    }))
}

// ── Shared error mapping ──────────────────────────────────────────────────────

/// A cursor that did not parse, or that names no position in this history.
///
/// 400 rather than 404: the cursor is something the client sent, and the fix is on its
/// side. The offending string is never repeated back.
fn malformed_cursor() -> ApiError {
    ApiError::invalid_request(
        "`cursor` is not a position in this node's audit history. Use a `next_cursor` \
         returned by this endpoint; cursors are opaque and must not be constructed or \
         modified.",
    )
}

/// Map a facade read failure, singling out the one case that is the caller's fault.
fn read_error(error: HaciendaError) -> ApiError {
    match &error {
        HaciendaError::Audit(AuditError::UnresolvableCursor { .. }) => malformed_cursor(),
        _ => ApiError::from(error),
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        routes::{build_router, tests::substitute_path_parameters, ROUTE_TABLE},
        state::{ApiLimits, ApiState},
    };
    use axum::{
        body::Body,
        http::{Request, Response},
    };
    use hacienda_core::{
        audit::{AuditStore, FileAuditStore, NodeId},
        auth::{authn::DevTokenResolver, AuthState},
        jobs::InMemoryJobStore,
        pii::PipelineConfig,
        HaciendaConfig, HaciendaFacade,
    };
    use std::{fs, path::Path, path::PathBuf, sync::Arc};
    use tower::ServiceExt;

    // ── The control corpus ────────────────────────────────────────────────────

    /// Values that must never appear in any response.
    ///
    /// Three are detected by the built-in patterns and therefore reach the audit chain
    /// (email, IBAN, US SSN); the fourth is an undetected token from the same document,
    /// there to catch an endpoint that echoes raw input rather than only detected spans.
    /// All four are deliberately unlike anything else in the codebase, so a match is a
    /// disclosure and never a coincidence.
    const CORPUS_VALUES: &[&str] = &[
        "zephyrine.quatrebarbes@corpus-temoin.example",
        "FR7630006000011234567890189",
        "271-82-4409",
        "OCTOPUS-MARMALADE-7731",
    ];

    fn corpus_document() -> String {
        format!(
            "Dossier {}: ecrire a {}, IBAN {}, SSN {}.",
            CORPUS_VALUES[3], CORPUS_VALUES[0], CORPUS_VALUES[1], CORPUS_VALUES[2]
        )
    }

    /// A token granting every capability, so the sweep is limited by the surface and not
    /// by what the token happens to hold.
    const ALL_CAPS_TOKEN: &str = "Bearer hcd_documents:process,pii:reveal,audit:read,\
                                  audit:export,review:decide,raw:extract_sweepsuffix";

    // ── State helpers ─────────────────────────────────────────────────────────

    fn state_from(facade: HaciendaFacade) -> ApiState {
        ApiState::new(
            Arc::new(facade),
            InMemoryJobStore::new().into_arc(),
            AuthState::new(Arc::new(DevTokenResolver)),
            ApiLimits::default(),
        )
    }

    /// Detection on, auditing on, in-memory chain.
    fn state_with_audit() -> ApiState {
        let config = HaciendaConfig::default().with_pii(PipelineConfig::default());
        state_from(HaciendaFacade::new(config).expect("facade"))
    }

    /// Detection off, so no audit store exists at all.
    fn state_without_audit() -> ApiState {
        state_from(HaciendaFacade::new(HaciendaConfig::default()).expect("facade"))
    }

    /// Detection on, auditing on, backed by a real directory so entries can be tampered
    /// with on disk. Returns the concrete store too, for rotation and file surgery.
    fn state_with_file_audit(root: &Path) -> (ApiState, Arc<FileAuditStore>, PathBuf) {
        let node = NodeId::new("corpus-node");
        let node_dir = root.join(node.as_str());
        let store = Arc::new(
            FileAuditStore::open(root, node, "corpus-config").expect("open the file store"),
        );
        let mut pii = PipelineConfig::default();
        pii.audit.config_hash = "corpus-config".into();
        let config = HaciendaConfig::default().with_pii(pii);
        let facade =
            HaciendaFacade::with_stores(config, Some(store.clone()), None, None).expect("facade");
        (state_from(facade), store, node_dir)
    }

    // ── HTTP helpers ──────────────────────────────────────────────────────────

    async fn get(app: &axum::Router, uri: &str, token: &str) -> Response<Body> {
        app.clone()
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri(uri)
                    .header("authorization", token)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap()
    }

    async fn body_text(response: Response<Body>) -> String {
        let bytes = axum::body::to_bytes(response.into_body(), 8 * 1024 * 1024)
            .await
            .expect("body");
        String::from_utf8_lossy(&bytes).into_owned()
    }

    /// Run the control corpus through `POST /v1/pii/redact`, which is what writes the
    /// audit entries the audit endpoints then serve.
    async fn process_the_corpus(app: &axum::Router) -> String {
        let body = serde_json::json!({ "text": corpus_document() }).to_string();
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/pii/redact")
                    .header("authorization", ALL_CAPS_TOKEN)
                    .header("content-type", "application/json")
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            response.status().as_u16(),
            200,
            "the corpus must be processed before the endpoints are swept"
        );
        body_text(response).await
    }

    // ── Step 4.1 — the test this whole spec exists for ────────────────────────

    /// Process a control corpus, then query **every** route in the table and assert that
    /// no response carries one of the corpus values.
    ///
    /// The route table is swept rather than listed, so a route added later is covered the
    /// day it is added rather than the day someone remembers to extend this test.
    ///
    /// # Why the sweep is checked for being non-vacuous
    ///
    /// A sweep that received 401s, 404s or empty bodies everywhere would pass while proving
    /// nothing. So the audit endpoints are asserted to have answered 200, and the entries
    /// endpoint is asserted to have returned the chain records the corpus produced. The
    /// test only means something if it is looking at real audit output.
    #[tokio::test]
    async fn no_endpoint_returns_corpus_plaintext() {
        let app = build_router(state_with_audit());

        let redacted = process_the_corpus(&app).await;
        for value in CORPUS_VALUES.iter().take(3) {
            assert!(
                !redacted.contains(value),
                "the redaction response leaked a detected corpus value: {redacted}"
            );
        }

        // Every table path, plus the export formats, which are distinct representations
        // of the same history and each need checking.
        let mut targets: Vec<String> = ROUTE_TABLE
            .iter()
            .map(|spec| substitute_path_parameters(spec.path))
            .collect();
        targets.push("/v1/audit/export?format=json".into());
        targets.push("/v1/audit/export?format=jsonl".into());
        targets.push("/v1/audit/export?format=csv".into());
        targets.push("/v1/audit/entries?limit=1000".into());

        let mut answered: std::collections::HashSet<&str> = std::collections::HashSet::new();
        for target in &targets {
            let response = get(&app, target, ALL_CAPS_TOKEN).await;
            let status = response.status().as_u16();
            let text = body_text(response).await;

            for value in CORPUS_VALUES {
                assert!(
                    !text.contains(value),
                    "GET {target} returned a corpus value ({value}) in its {status} \
                     response: {text}"
                );
            }

            if target.starts_with("/v1/audit/") {
                assert_eq!(
                    status, 200,
                    "GET {target} must answer, got {status}: {text}"
                );
                let path = target.split('?').next().unwrap();
                answered.insert(path);
            }
        }

        // Derived from the table rather than counted by hand: an audit route added later
        // is required to have answered here without anyone editing this assertion.
        for spec in ROUTE_TABLE
            .iter()
            .filter(|spec| spec.path.starts_with("/v1/audit/"))
        {
            assert!(
                answered.contains(spec.path),
                "{} was in the route table but did not answer 200 during the sweep, so \
                 nothing was actually inspected for it",
                spec.path
            );
        }

        // Non-vacuity: the entries endpoint really served the chain records the corpus
        // produced, so "no corpus value in the body" is a statement about audit content
        // and not about an empty response.
        let page: serde_json::Value = serde_json::from_str(
            &body_text(get(&app, "/v1/audit/entries", ALL_CAPS_TOKEN).await).await,
        )
        .expect("the entries response is JSON");
        let entries = page["entries"].as_array().expect("an entries array");
        assert!(
            entries.len() >= 3,
            "the corpus must have produced at least one entry per detected value, got {}",
            entries.len()
        );
        assert!(
            entries
                .iter()
                .all(|entry| entry["span_hash"].as_str().is_some_and(|h| h.len() == 64)),
            "every entry must carry a blake3 digest in place of the span"
        );
        assert_eq!(page["scope"], "this_node");
    }

    // ── Step 4.2 — a broken chain is an answer ────────────────────────────────

    /// Tamper with a sealed entry on disk, then ask `/v1/audit/verify`: the response must
    /// be a 200 that names the record, not an opaque 500.
    #[tokio::test]
    async fn verify_names_the_broken_entry() {
        let dir = TempDir::new("verify-broken");
        let (state, store, node_dir) = state_with_file_audit(dir.path());
        let app = build_router(state);

        process_the_corpus(&app).await;

        // Seal the segment: `verify` re-reads sealed segments from disk, which is what
        // makes on-disk tampering detectable in the first place.
        let seal = store.rotate().await.expect("rotate");
        assert!(seal.entry_count >= 3, "the corpus must fill the segment");

        let path = node_dir.join(format!("{}.jsonl", seal.segment_id));
        let original = fs::read_to_string(&path).expect("read the sealed segment");
        let mut records: Vec<serde_json::Value> = original
            .lines()
            .map(|line| serde_json::from_str(line).expect("a JSONL record"))
            .collect();

        let tampered_index = 1usize;
        let recorded_hash = records[tampered_index]["chain_hash"]
            .as_str()
            .expect("a chain hash")
            .to_owned();
        records[tampered_index]["category"] = serde_json::json!("CreditCard");

        let rewritten: String = records
            .iter()
            .map(|record| format!("{record}\n"))
            .collect::<Vec<_>>()
            .concat();
        fs::write(&path, rewritten).expect("rewrite the sealed segment");

        let response = get(&app, "/v1/audit/verify", ALL_CAPS_TOKEN).await;
        assert_eq!(
            response.status().as_u16(),
            200,
            "a broken chain is an answer to the question that was asked, not a server \
             failure"
        );

        let body: serde_json::Value =
            serde_json::from_str(&body_text(response).await).expect("JSON");
        assert_eq!(body["verified"], false);
        assert_eq!(body["failure"]["kind"], "chain_integrity");
        assert_eq!(body["failure"]["entry_index"], tampered_index as u64);
        assert_eq!(
            body["failure"]["actual_hash"], recorded_hash,
            "the response must name the offending record by the hash it carries on disk"
        );
    }

    /// The counterpart: an untampered chain verifies. Without this, an endpoint hard-wired
    /// to report a break would satisfy the test above.
    #[tokio::test]
    async fn verify_reports_an_intact_chain_as_verified() {
        let dir = TempDir::new("verify-intact");
        let (state, store, _) = state_with_file_audit(dir.path());
        let app = build_router(state);

        process_the_corpus(&app).await;
        store.rotate().await.expect("rotate");

        let body: serde_json::Value = serde_json::from_str(
            &body_text(get(&app, "/v1/audit/verify", ALL_CAPS_TOKEN).await).await,
        )
        .expect("JSON");

        assert_eq!(body["verified"], true);
        assert!(body.get("failure").is_none(), "no failure on a clean chain");
    }

    // ── Step 4.3 — the tip is reachable without audit:read ────────────────────

    /// D-P2-1: a caller holding only `documents:process` can obtain the chain evidence for
    /// its own result.
    #[tokio::test]
    async fn tip_is_reachable_with_documents_process_alone() {
        let app = build_router(state_with_audit());
        let token = "Bearer hcd_documents:process_tipsuffix";

        let response = get(&app, "/v1/audit/tip", token).await;
        let status = response.status().as_u16();
        assert_ne!(status, 401, "the tip must not require another identity");
        assert_ne!(status, 403, "the tip must not require audit:read");
        assert_eq!(status, 200, "unexpected status for the tip: {status}");

        let body: serde_json::Value =
            serde_json::from_str(&body_text(response).await).expect("JSON");
        assert_eq!(body["scope"], "this_node");
        assert_eq!(
            body["tip"].as_str().map(str::len),
            Some(64),
            "the tip is an opaque blake3 digest"
        );
    }

    /// The same token must **not** open the rest of the audit surface, or the test above
    /// would be evidence of a missing guard rather than of a deliberate exception.
    #[tokio::test]
    async fn documents_process_alone_does_not_open_the_rest_of_the_audit_surface() {
        let app = build_router(state_with_audit());
        let token = "Bearer hcd_documents:process_tipsuffix";

        for path in [
            "/v1/audit/entries",
            "/v1/audit/verify",
            "/v1/audit/seals",
            "/v1/audit/export",
        ] {
            let status = get(&app, path, token).await.status().as_u16();
            assert_eq!(status, 403, "{path} must require an audit capability");
        }
    }

    // ── Step 4.4 — the two audit capabilities are distinct ────────────────────

    /// A token holding `audit:read` alone must be refused at `/v1/audit/export`.
    ///
    /// Reading the chain on the server and carrying a copy of it out of the building are
    /// different acts; the capability model separates them and this asserts the separation
    /// survives the HTTP layer.
    #[tokio::test]
    async fn export_requires_audit_export_not_audit_read() {
        let app = build_router(state_with_audit());
        process_the_corpus(&app).await;

        let read_only = "Bearer hcd_audit:read_readsuffix";
        assert_eq!(
            get(&app, "/v1/audit/export", read_only).await.status(),
            403,
            "audit:read alone must not export"
        );

        // Paired assertions, so a blanket 403 regression cannot satisfy the one above:
        // the same token works where it should, and audit:export works on the export.
        assert_eq!(
            get(&app, "/v1/audit/entries", read_only).await.status(),
            200,
            "audit:read must still read entries"
        );
        assert_eq!(
            get(
                &app,
                "/v1/audit/export",
                "Bearer hcd_audit:export_expsuffix"
            )
            .await
            .status(),
            200,
            "audit:export must export"
        );
    }

    // ── The two artefacts must not be confusable ──────────────────────────────

    /// A CSV export must announce that it is not evidence, and a JSON one that it is —
    /// in the header, in the filename that survives being saved, and in the content type.
    #[tokio::test]
    async fn csv_export_is_labelled_as_not_evidence_and_json_as_an_envelope() {
        let app = build_router(state_with_audit());
        process_the_corpus(&app).await;

        let csv = get(&app, "/v1/audit/export?format=csv", ALL_CAPS_TOKEN).await;
        let headers = csv.headers().clone();
        assert_eq!(headers["x-hacienda-audit-evidence"], "none");
        assert_eq!(headers["x-hacienda-audit-scope"], "this-node");
        let disposition = headers["content-disposition"].to_str().unwrap();
        assert!(
            disposition.contains("NOT-EVIDENCE"),
            "the saved filename must carry the warning, because that is what survives \
             being forwarded: {disposition}"
        );
        assert!(headers["content-type"].to_str().unwrap().contains("csv"));
        let csv_body = body_text(csv).await;
        assert!(
            csv_body.lines().count() >= 4,
            "a header row plus one row per corpus detection"
        );

        for (format, extension) in [("json", "json"), ("jsonl", "jsonl")] {
            let response = get(
                &app,
                &format!("/v1/audit/export?format={format}"),
                ALL_CAPS_TOKEN,
            )
            .await;
            let headers = response.headers().clone();
            assert_eq!(headers["x-hacienda-audit-evidence"], "envelope");
            let disposition = headers["content-disposition"].to_str().unwrap();
            assert!(
                disposition.contains(&format!("evidence-envelope.{extension}")),
                "{format}: {disposition}"
            );
        }
    }

    /// An unspecified `format` must yield the verifiable artefact, never the tabular one.
    ///
    /// The verifiable artefact is currently a flat, chain-ordered JSON array of entries
    /// (`hacienda_core::audit::export::export_json`) — not a `{version, segments}`
    /// envelope grouped by segment with embedded seals. Building that grouped shape is
    /// a real gap against this module's own doc comment and `hacienda-api/README.md`,
    /// tracked separately; this test pins today's actual, offline-verifiable-by-chain-
    /// hash-sequence behaviour rather than asserting a shape the code doesn't produce.
    #[tokio::test]
    async fn export_defaults_to_the_verifiable_envelope() {
        let app = build_router(state_with_audit());
        process_the_corpus(&app).await;

        let response = get(&app, "/v1/audit/export", ALL_CAPS_TOKEN).await;
        assert_eq!(response.headers()["x-hacienda-audit-evidence"], "envelope");

        let body: serde_json::Value =
            serde_json::from_str(&body_text(response).await).expect("valid JSON");
        assert!(
            body.as_array().is_some_and(|entries| !entries.is_empty()),
            "the default export must be a non-empty JSON array of chain entries, got {body:?}"
        );
    }

    #[tokio::test]
    async fn an_unknown_export_format_is_refused() {
        let app = build_router(state_with_audit());
        let response = get(&app, "/v1/audit/export?format=xlsx", ALL_CAPS_TOKEN).await;
        assert_eq!(response.status(), 400);
    }

    // ── Paging ────────────────────────────────────────────────────────────────

    /// `limit=0` must be refused rather than answered with an empty page.
    ///
    /// Core returns an empty page with no cursor for it, which a client cannot tell from
    /// "you are caught up" — so an uninitialised paging variable would read as an empty
    /// history. There is no reading of "give me zero entries" the caller could have meant.
    #[tokio::test]
    async fn a_zero_limit_is_refused_rather_than_read_as_caught_up() {
        let app = build_router(state_with_audit());
        process_the_corpus(&app).await;

        let response = get(&app, "/v1/audit/entries?limit=0", ALL_CAPS_TOKEN).await;
        assert_eq!(response.status(), 400);
        let text = body_text(response).await;
        assert!(
            text.contains("limit"),
            "the error must name the parameter: {text}"
        );
    }

    /// An over-large limit is clamped rather than refused: unlike zero, it has an
    /// unambiguous safe reading, and the caller's page-until-empty loop absorbs it.
    #[tokio::test]
    async fn an_over_large_limit_is_clamped_and_still_answers() {
        let app = build_router(state_with_audit());
        process_the_corpus(&app).await;

        let response = get(&app, "/v1/audit/entries?limit=999999", ALL_CAPS_TOKEN).await;
        assert_eq!(response.status(), 200);
    }

    /// The documented paging contract: a non-empty page always carries a cursor, and the
    /// caller stops when it receives an **empty** page — not when the cursor goes null.
    #[tokio::test]
    async fn paging_stops_on_an_empty_page_and_never_repeats_an_entry() {
        let app = build_router(state_with_audit());
        process_the_corpus(&app).await;

        let mut seen: Vec<String> = Vec::new();
        let mut cursor: Option<String> = None;

        for _ in 0..10 {
            let uri = match &cursor {
                Some(c) => format!("/v1/audit/entries?limit=1&cursor={c}"),
                None => "/v1/audit/entries?limit=1".to_string(),
            };
            let page: serde_json::Value =
                serde_json::from_str(&body_text(get(&app, &uri, ALL_CAPS_TOKEN).await).await)
                    .expect("JSON");
            let entries = page["entries"].as_array().expect("entries").clone();
            if entries.is_empty() {
                assert!(
                    page["next_cursor"].is_null(),
                    "an empty page carries no cursor"
                );
                assert!(!seen.is_empty(), "the corpus produced entries to page over");
                let unique: std::collections::HashSet<&String> = seen.iter().collect();
                assert_eq!(
                    unique.len(),
                    seen.len(),
                    "paging repeated an entry: {seen:?}"
                );
                return;
            }
            assert!(
                page["next_cursor"].is_string(),
                "a non-empty page must always hand back a resumable position"
            );
            cursor = Some(page["next_cursor"].as_str().unwrap().to_owned());
            for entry in entries {
                seen.push(entry["id"].as_str().expect("an id").to_owned());
            }
        }

        panic!("paging did not terminate within ten pages");
    }

    /// A cursor the client made up must be refused, and the refusal must not echo it back.
    #[tokio::test]
    async fn a_forged_cursor_is_refused_without_being_echoed() {
        let app = build_router(state_with_audit());
        process_the_corpus(&app).await;

        for forged in ["not-a-cursor", "00000000-0000-4000-8000-000000000000:9"] {
            let response = get(
                &app,
                &format!("/v1/audit/entries?cursor={forged}"),
                ALL_CAPS_TOKEN,
            )
            .await;
            assert_eq!(response.status(), 400, "forged cursor {forged}");
            let text = body_text(response).await;
            assert!(
                !text.contains(forged),
                "the refusal echoed the client's input back: {text}"
            );
        }
    }

    /// An unsupported filter must be a refusal, not a silently unfiltered answer.
    ///
    /// A caller who asks for `?principal=alice` and receives the whole chain has been
    /// handed a superset of what they asked for with nothing to tell them so — on a record
    /// whose purpose is to answer "who saw this value", that is worse than an error.
    #[tokio::test]
    async fn an_unsupported_filter_is_refused_rather_than_ignored() {
        let app = build_router(state_with_audit());
        process_the_corpus(&app).await;

        for query in ["principal=alice", "category=Email", "from=2026-01-01"] {
            let status = get(&app, &format!("/v1/audit/entries?{query}"), ALL_CAPS_TOKEN)
                .await
                .status();
            assert_eq!(status, 400, "?{query} must be refused, not ignored");
        }

        assert_eq!(
            get(&app, "/v1/audit/seals?limit=10", ALL_CAPS_TOKEN)
                .await
                .status(),
            400,
            "an endpoint with no parameters must reject the ones it does not have"
        );
    }

    // ── No chain is not an empty chain ────────────────────────────────────────

    /// With auditing off, every audit endpoint must say there is no chain — never report
    /// an empty one, and never report a verified one.
    #[tokio::test]
    async fn endpoints_report_an_absent_chain_rather_than_an_empty_one() {
        let app = build_router(state_without_audit());

        for path in [
            "/v1/audit/entries",
            "/v1/audit/verify",
            "/v1/audit/seals",
            "/v1/audit/export",
            "/v1/audit/tip",
        ] {
            let response = get(&app, path, ALL_CAPS_TOKEN).await;
            assert_eq!(
                response.status(),
                404,
                "{path} must not answer as if a chain existed"
            );
            let text = body_text(response).await;
            assert!(
                text.contains("not enabled"),
                "{path} must say why there is nothing to read: {text}"
            );
        }
    }

    // ── TempDir RAII helper ───────────────────────────────────────────────────
    // Same six lines as `audit/store_file.rs`'s test module; not worth a shared crate.

    struct TempDir(PathBuf);

    impl TempDir {
        fn new(tag: &str) -> Self {
            let path = std::env::temp_dir().join(format!(
                "hacienda-api-audit-{tag}-{}",
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
}
