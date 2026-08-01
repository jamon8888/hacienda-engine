//! Request and response data transfer objects.
//!
//! # Wire safety
//!
//! `xberg::ExtractInput` is not accepted from the wire. Its `Uri` variant accepts
//! filesystem paths and HTTP URLs — handing it to a deserializer would give any
//! authenticated caller arbitrary local file reads and SSRF against the host network.
//!
//! All document input arrives as inline base64-encoded bytes via [`DocumentInput`].
//! The handler converts to `ExtractInput::from_bytes`, which is the only `ExtractInput`
//! constructor this crate is allowed to call.

use data_encoding::BASE64;
use hacienda_core::pii::{EntitySource, MergedEntity, PiiCategory};
use serde::{Deserialize, Serialize};

// ── Requests ─────────────────────────────────────────────────────────────────

/// One document in a `POST /v1/documents` or `POST /v1/documents/async` request.
///
/// `content_base64` is the only accepted form of document content. There is no
/// `uri`, `path`, or `file` field — those would be SSRF / path traversal.
///
/// `deny_unknown_fields` matters here specifically: without it, a client probing for a
/// passthrough with `{"uri": "file:///etc/passwd", ...}` gets a 200, which reads as
/// "accepted" and invites a follow-up. Rejecting the field says the surface does not
/// exist, and — more usefully for legitimate clients — a misspelled field becomes an
/// error rather than a silently ignored setting.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DocumentInput {
    /// Original filename for display purposes only. Never used to open a file.
    pub filename: Option<String>,
    /// MIME type hint, e.g. `application/pdf` or `text/plain`.
    pub mime_type: String,
    /// Document bytes encoded as standard base64.
    pub content_base64: String,
}

impl DocumentInput {
    /// Decode the base64 content. Returns an error message (never the content) on failure.
    pub fn decode_bytes(&self) -> Result<Vec<u8>, String> {
        BASE64
            .decode(self.content_base64.as_bytes())
            .map_err(|_| "content_base64 is not valid base64".to_string())
    }
}

/// Body for `POST /v1/documents` and `POST /v1/documents/async`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProcessDocumentsRequest {
    pub documents: Vec<DocumentInput>,
}

/// Body for `POST /v1/pii/scan`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScanTextRequest {
    pub text: String,
    /// When `true`, detected span text is included in the response.
    ///
    /// Requires the caller to also hold `pii:reveal`. Absent or `false` is the safe
    /// default: the caller learns what categories were found, not their values.
    #[serde(default)]
    pub include_text: bool,
}

/// Body for `POST /v1/pii/redact`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RedactTextRequest {
    pub text: String,
}

// ── Response DTOs ─────────────────────────────────────────────────────────────

/// A detected entity in a scan response.
///
/// `text` is `None` unless `include_text` was set to `true` **and** the caller
/// held `pii:reveal`. The clear-in-core guarantee means this DTO carries whatever
/// the facade returned — suppression happened upstream.
#[derive(Debug, Serialize)]
pub struct EntityDto {
    pub category: PiiCategory,
    pub start: u32,
    pub end: u32,
    pub confidence: f32,
    pub source: EntitySource,
    /// Present only when `include_text=true` was granted.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
}

impl From<MergedEntity> for EntityDto {
    fn from(entity: MergedEntity) -> Self {
        // `text` is empty-string when suppressed by the facade. Convert empty → None
        // so the field is absent in the JSON response rather than `"text": ""`.
        let text = if entity.text.is_empty() {
            None
        } else {
            Some(entity.text)
        };
        Self {
            category: entity.category,
            start: entity.start,
            end: entity.end,
            confidence: entity.confidence,
            source: entity.source,
            text,
        }
    }
}

/// Response from `POST /v1/pii/scan`.
#[derive(Debug, Serialize)]
pub struct ScanTextResponse {
    pub entities: Vec<EntityDto>,
    pub document_count: u32,
    pub processing_time_ms: u64,
    /// Current audit chain tip. `null` when auditing is disabled.
    pub audit_chain_tip: Option<String>,
}

/// Response from `POST /v1/pii/redact`.
#[derive(Debug, Serialize)]
pub struct RedactTextResponse {
    pub redacted_text: String,
    pub entity_count: usize,
    pub processing_time_ms: u64,
    /// Current audit chain tip. `null` when auditing is disabled.
    pub audit_chain_tip: Option<String>,
}

/// A single document's result within a `POST /v1/documents` response.
#[derive(Debug, Serialize)]
pub struct DocumentResult {
    /// Redacted content.
    pub content: String,
    /// Detected PII categories and positions. Span text is never included here —
    /// `POST /v1/documents` does not support `include_text`.
    pub entities: Vec<EntityDto>,
}

/// Response from `POST /v1/documents`.
#[derive(Debug, Serialize)]
pub struct ProcessDocumentsResponse {
    pub documents: Vec<DocumentResult>,
    pub processing_time_ms: u64,
    /// Current audit chain tip. `null` when auditing is disabled.
    pub audit_chain_tip: Option<String>,
}

/// Response from `POST /v1/documents/async` (202 Accepted).
#[derive(Debug, Serialize)]
pub struct AsyncJobResponse {
    pub job_id: String,
}

/// Response from `GET /v1/jobs/{id}`.
#[derive(Debug, Serialize)]
pub struct JobResponse {
    pub id: String,
    pub status: hacienda_core::jobs::JobStatus,
    pub created_at: String,
    pub updated_at: String,
    /// Populated when status is `succeeded`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<serde_json::Value>,
    /// Populated when status is `failed`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl From<hacienda_core::jobs::Job> for JobResponse {
    fn from(job: hacienda_core::jobs::Job) -> Self {
        let result = job
            .result_json
            .as_deref()
            .and_then(|s| serde_json::from_str(s).ok());
        Self {
            id: job.id,
            status: job.status,
            created_at: job.created_at,
            updated_at: job.updated_at,
            result,
            // Do not forward internal error strings to the client verbatim;
            // a short failure reason is acceptable but must not contain document content.
            // The store records what the pipeline returned. We take it as-is here because
            // the async handler writes the error string (see handlers/documents.rs) and
            // is responsible for not writing PII into it.
            error: job.error,
        }
    }
}

/// Response from `GET /v1/pii/config`.
///
/// An explicit allowlist of fields rather than a derived `Serialize` on `PipelineConfig`.
/// This prevents a newly added field — in particular key material — from being exposed
/// unintentionally.
#[derive(Debug, Serialize)]
pub struct PiiConfigResponse {
    pub enabled: bool,
    pub regex_first: bool,
    pub model_threshold_default: f32,
    pub merge_overlap_threshold: f32,
    pub redaction_mode: String,
    /// `true` when a statistical NER model is configured (even if weights are not yet
    /// loaded). `GET /v1/pii/config` must report this honestly — §13 of the spec names
    /// silently running regex-only while advertising model detection as a worse failure
    /// than not shipping the endpoint.
    pub model_enabled: bool,
    pub audit_enabled: bool,
}

// ── Audit DTOs ────────────────────────────────────────────────────────────────
//
// Why these are hand-written DTOs rather than `Serialize` on the core types.
//
// The audit record is designed to hold no document content — only blake3 digests, spans
// lengths and categories — and `no_endpoint_returns_corpus_plaintext` asserts that at the
// HTTP boundary. An explicit field list is what keeps the assertion meaningful over time:
// a field added to `hacienda_core::audit::AuditEntry` cannot reach the wire until someone
// adds it here too, and that edit is where the question "does this carry corpus content?"
// gets asked. Deriving `Serialize` on the core type would forward new fields silently.
//
// `GET /v1/audit/export` is the deliberate exception: it returns core's own bytes
// verbatim, because an evidence envelope only verifies offline if it is byte-for-byte
// what the verifier expects. See `handlers::audit::audit_export`.

/// Whose audit chain a response describes.
///
/// Serialised on **every** audit response, and the reason it exists is that there is only
/// one possible value today. Segments are per-writer and there is no total order between
/// writers, so a server can only ever speak for its own node. Saying so in the payload —
/// rather than in prose a client never reads — is what stops "the audit entries" being
/// read as the deployment's when it means this node's.
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AuditScope {
    /// The chain held by the node that served the request, and no other.
    ThisNode,
}

/// One audit entry on the wire.
///
/// Carries no span text by construction: `span_hash` is a blake3 digest and `span_length`
/// a count. That is a property of the core entry, and this DTO is where it is pinned.
#[derive(Debug, Serialize)]
pub struct AuditEntryDto {
    pub id: String,
    pub timestamp: String,
    /// The PII category, e.g. `Email` — never the value that was found.
    pub category: String,
    pub action: hacienda_core::audit::RedactionAction,
    /// blake3 digest of the span. The span itself is never recorded anywhere.
    pub span_hash: String,
    pub span_length: u32,
    pub confidence: Option<f32>,
    pub source: hacienda_core::audit::EntitySource,
    pub pipeline_version: String,
    pub config_hash: String,
    /// The authenticated principal, or `null` for an in-process caller.
    pub principal: Option<String>,
    /// blake3 over the previous entry's chain hash and this entry's identifying fields.
    pub chain_hash: String,
}

impl From<hacienda_core::audit::AuditEntry> for AuditEntryDto {
    fn from(entry: hacienda_core::audit::AuditEntry) -> Self {
        Self {
            id: entry.id,
            timestamp: entry.timestamp,
            category: entry.category,
            action: entry.action,
            span_hash: entry.span_hash,
            span_length: entry.span_length,
            confidence: entry.confidence,
            source: entry.source,
            pipeline_version: entry.pipeline_version,
            config_hash: entry.config_hash,
            principal: entry.principal,
            chain_hash: entry.chain_hash,
        }
    }
}

/// One segment seal on the wire.
#[derive(Debug, Serialize)]
pub struct SegmentSealDto {
    pub segment_id: String,
    /// The writer that produced the segment. This is where a client learns the node
    /// identity behind [`AuditScope::ThisNode`].
    pub node_id: String,
    pub config_hash: String,
    pub prev_seal_hash: Option<String>,
    pub sealed_tip: String,
    pub entry_count: u64,
    pub opened_at: String,
    pub sealed_at: String,
    pub seal_hash: String,
}

impl From<hacienda_core::audit::SegmentSeal> for SegmentSealDto {
    fn from(seal: hacienda_core::audit::SegmentSeal) -> Self {
        Self {
            segment_id: seal.segment_id,
            node_id: seal.node_id,
            config_hash: seal.config_hash,
            prev_seal_hash: seal.prev_seal_hash,
            sealed_tip: seal.sealed_tip,
            entry_count: seal.entry_count,
            opened_at: seal.opened_at,
            sealed_at: seal.sealed_at,
            seal_hash: seal.seal_hash,
        }
    }
}

/// Response from `GET /v1/audit/entries`.
///
/// Named for its scope on purpose. A type called `AuditPage` invites the reading "the
/// audit entries"; this one cannot be read as anything but one node's.
///
/// # Paging contract
///
/// `next_cursor` is present whenever the page is non-empty and absent only when the page
/// is empty, so a client pages **until it receives an empty page** — not until
/// `next_cursor` is `null`. An append-only chain is never finished, only momentarily
/// caught up, and a caller that reached the end still needs a resumable position to
/// follow what comes next. See `hacienda_core::audit::AuditPage::next`.
#[derive(Debug, Serialize)]
pub struct NodeAuditPage {
    pub scope: AuditScope,
    pub entries: Vec<AuditEntryDto>,
    /// Opaque. Hand it back as `?cursor=`; never parse, construct, or arithmetic on it.
    pub next_cursor: Option<String>,
}

/// Response from `GET /v1/audit/seals`.
#[derive(Debug, Serialize)]
pub struct NodeSealsResponse {
    pub scope: AuditScope,
    pub seals: Vec<SegmentSealDto>,
}

/// Response from `GET /v1/audit/tip`.
#[derive(Debug, Serialize)]
pub struct AuditTipResponse {
    pub scope: AuditScope,
    /// The chain head, an opaque blake3 digest.
    pub tip: String,
}

/// Which of the chain's integrity checks failed.
///
/// The variants are the tamper conditions, not every way a read can go wrong: an I/O
/// failure is not a verdict on the chain and is reported as a 500 instead, because
/// answering "broken" when the truth is "could not be read" would raise a tamper alarm
/// over a full disk.
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum VerifyFailureKind {
    /// An entry's recorded hash does not follow its predecessor's.
    ChainIntegrity,
    /// A seal's own hash does not match the fields it seals.
    SegmentIntegrity,
    /// A seal does not link to the seal before it — a segment was inserted, deleted or
    /// reordered.
    SegmentLink,
    /// A sealed segment holds a different number of entries than its seal recorded.
    SegmentEntryCount,
    /// An entry claims a configuration this chain was not written under.
    ConfigMismatch,
}

/// Where verification failed, in machine-readable form.
///
/// Every field here is a hash, an index, a segment id, or a configuration hash. None of
/// them can carry document content.
///
/// # What "names the entry" means, precisely
///
/// For [`VerifyFailureKind::ChainIntegrity`], `entry_index` is the entry's sequence number
/// **within its segment** and `actual_hash` is that entry's own recorded `chain_hash` —
/// together they identify the record. `segment_id` is populated for the seal-level
/// failures, which report it; core's entry-chain check does not carry the segment through,
/// so it is `null` there. That is a real limitation of the underlying error, stated rather
/// than papered over with a guess.
#[derive(Debug, Serialize)]
pub struct VerifyFailure {
    pub kind: VerifyFailureKind,
    /// The core error, rendered. Contains only identifiers and hashes — no path, no
    /// document content. Built from the classified variants alone; an unclassified error
    /// never reaches this field.
    pub detail: String,
    pub segment_id: Option<String>,
    pub entry_index: Option<u64>,
    pub expected_hash: Option<String>,
    pub actual_hash: Option<String>,
}

/// Response from `GET /v1/audit/verify`.
///
/// A broken chain is a **200 with `verified: false`**, not a 500. The question the caller
/// asked — "is this chain intact?" — was answered; a 500 would report that the server
/// failed to answer, which is a different and less useful statement, and one that hides
/// the identity of the offending record behind a generic error envelope.
#[derive(Debug, Serialize)]
pub struct VerifyResponse {
    pub scope: AuditScope,
    pub verified: bool,
    /// Present exactly when `verified` is `false`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub failure: Option<VerifyFailure>,
}

/// Response from `GET /health`.
#[derive(Debug, Serialize)]
pub struct HealthResponse {
    pub status: &'static str,
}

/// Response from `GET /version`.
#[derive(Debug, Serialize)]
pub struct VersionResponse {
    pub version: &'static str,
}

/// Response from `GET /info`.
#[derive(Debug, Serialize)]
pub struct InfoResponse {
    pub name: &'static str,
    pub version: &'static str,
    pub description: &'static str,
}
