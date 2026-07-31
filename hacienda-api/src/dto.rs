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
    /// The active Tier 0 vertical's id, or `None` when no vertical is configured.
    ///
    /// Deliberately exposed: unlike `model_dir`/`lora_adapter_dir` (filesystem paths,
    /// host topology) a vertical id and its label set are not secret or
    /// host-identifying — they describe *what the pipeline is configured to detect*,
    /// which a caller integrating against `/v1/pii/redact` needs in order to know what
    /// categories to expect back. See the implementation plan's Task 2.4.
    pub vertical_id: Option<String>,
    /// The active vertical's zero-shot labels. Empty when no vertical is configured.
    pub vertical_labels: Vec<String>,
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
