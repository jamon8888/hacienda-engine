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
    /// Optional client-supplied document identity, enabling versioning
    /// (`GET /v1/documents/{id}/versions`, `GET /v1/documents/{id}`, `/diff`). Mirrors
    /// xberg-sdks' documented `document_id` field (`CHANGELOG.md` `[0.3.0]`). Omit to
    /// process without recording a version — requires `ApiState::version_store` to be
    /// configured when present, or the request is rejected with 400.
    #[serde(default)]
    pub document_id: Option<uuid::Uuid>,
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

/// Body for `POST /v1/pii/reveal`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RevealTokenRequest {
    /// A pseudonym token previously returned by a redaction operation.
    /// Format: `[CATEGORY:key_id:base32_ciphertext]`
    pub token: String,
}

/// Body for `POST /v1/review/{id}/decide`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewDecideRequest {
    pub decision: String, // "approve", "reject", "modify"
    pub reviewer: String,
    pub comment: String,
}

/// Body for `POST /v1/auth/keys`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IssueKeyRequest {
    /// The owning principal for the new key (a service name or user id).
    pub owner: String,
    /// Capability names in wire form, e.g. `"documents:process"`. Parsed via
    /// `Capability::from_str`; an unrecognised name is a 400, never a key issued
    /// with fewer capabilities than requested.
    pub capabilities: Vec<String>,
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

/// Response from `POST /v1/pii/reveal`.
#[derive(Debug, Serialize)]
pub struct RevealTokenResponse {
    /// The normalised plaintext value behind the token.
    pub plaintext: String,
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
    /// Echoes the request's `document_id` when versioning was requested.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub document_id: Option<uuid::Uuid>,
    /// The server-assigned 1-based version sequence, present alongside `document_id`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version_sequence: Option<i64>,
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

/// Query parameters for `GET /v1/jobs`.
#[derive(Debug, Deserialize)]
pub struct JobListQuery {
    /// Filter to jobs in this status. Omit to list all statuses.
    #[serde(default)]
    pub status: Option<hacienda_core::jobs::JobStatus>,
    /// Maximum jobs to return. Defaults to 50, capped at 200.
    #[serde(default)]
    pub limit: Option<usize>,
    /// Number of matching jobs to skip before the returned page. Defaults to 0.
    #[serde(default)]
    pub offset: Option<usize>,
}

/// Response from `GET /v1/jobs`.
///
/// `total` is the count of jobs matching `status` (and visible to the caller) before
/// `limit`/`offset` are applied, so a client can tell whether more pages remain.
#[derive(Debug, Serialize)]
pub struct JobListResponse {
    pub jobs: Vec<JobResponse>,
    pub total: usize,
}

/// Response from `GET /v1/jobs/{id}/result`.
///
/// Deliberately distinct from [`JobResponse`]: a polling loop that only needs to know
/// whether a job is done should not pay to deserialize a potentially large `result`
/// payload on every poll. This endpoint is for the caller that already knows the job is
/// (or might be) finished and wants the payload; `GET /v1/jobs/{id}` remains the
/// lightweight status-only poll.
#[derive(Debug, Serialize)]
pub struct JobResultResponse {
    pub status: hacienda_core::jobs::JobStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Audit entry DTO for wire serialization.
#[derive(Debug, Serialize)]
pub struct AuditEntryDto {
    pub id: String,
    pub category: String,
    pub action: String,
    pub span_hash: String,
    pub span_length: u64,
    pub confidence: Option<f64>,
    pub source: String,
    pub pipeline_version: String,
    pub config_hash: String,
    pub principal: Option<String>,
    pub chain_hash: String,
    pub created_at: String,
}

impl From<hacienda_core::audit::entry::AuditEntry> for AuditEntryDto {
    fn from(e: hacienda_core::audit::entry::AuditEntry) -> Self {
        let action = match e.action {
            hacienda_core::audit::entry::RedactionAction::Mask => "mask".to_string(),
            hacienda_core::audit::entry::RedactionAction::Hash => "hash".to_string(),
            hacienda_core::audit::entry::RedactionAction::Pseudonymize => {
                "pseudonymize".to_string()
            }
            hacienda_core::audit::entry::RedactionAction::Remove => "remove".to_string(),
            hacienda_core::audit::entry::RedactionAction::Reveal => "reveal".to_string(),
            hacienda_core::audit::entry::RedactionAction::Custom(s) => s,
        };
        Self {
            id: e.id,
            category: e.category,
            action,
            span_hash: e.span_hash,
            span_length: e.span_length as u64,
            confidence: e.confidence.map(|c| c as f64),
            source: e.source.to_string(),
            pipeline_version: e.pipeline_version,
            config_hash: e.config_hash,
            principal: e.principal,
            chain_hash: e.chain_hash,
            created_at: e.timestamp,
        }
    }
}

/// Review item DTO for wire serialization.
#[derive(Debug, Serialize)]
pub struct ReviewItemDto {
    pub id: String,
    pub text_snippet: String,
    pub category: String,
    pub start: u32,
    pub end: u32,
    pub confidence: f32,
    pub source: String,
    pub status: String,
    pub assigned_reviewer: Option<String>,
    pub decided_by: Option<String>,
    pub decided_at: Option<String>,
    pub decision: Option<String>,
    pub comment: Option<String>,
    pub deadline: Option<String>,
    pub created_at: String,
}

impl From<hacienda_core::review::types::ReviewQueueItem> for ReviewItemDto {
    fn from(item: hacienda_core::review::types::ReviewQueueItem) -> Self {
        let decision = item.decision.map(|d| {
            match d {
                hacienda_core::review::types::ReviewDecision::Approve => "approve",
                hacienda_core::review::types::ReviewDecision::Reject => "reject",
                hacienda_core::review::types::ReviewDecision::Modify => "modify",
            }
            .to_string()
        });
        Self {
            id: item.id,
            text_snippet: item.text_snippet,
            category: item.category,
            start: item.start,
            end: item.end,
            confidence: item.confidence,
            source: item.source,
            status: item.status.to_string(),
            assigned_reviewer: item.assigned_reviewer,
            decided_by: item.decided_by,
            decided_at: item.decided_at,
            decision,
            comment: item.comment,
            deadline: item.deadline,
            created_at: item.created_at,
        }
    }
}

/// Response from `GET /v1/audit`.
#[derive(Debug, Serialize)]
pub struct AuditResponse {
    pub entries: Vec<AuditEntryDto>,
    pub audit_chain_tip: Option<String>,
}

/// Response from `GET /v1/audit/verify`.
#[derive(Debug, Serialize)]
pub struct AuditVerifyResponse {
    pub valid: bool,
    pub audit_chain_tip: Option<String>,
}

/// Response from `GET /v1/review`.
#[derive(Debug, Serialize)]
pub struct ReviewResponse {
    pub items: Vec<ReviewItemDto>,
    pub audit_chain_tip: Option<String>,
}

/// Response from `POST /v1/review/{id}/decide`.
#[derive(Debug, Serialize)]
pub struct ReviewDecideResponse {
    pub item: ReviewItemDto,
    pub audit_chain_tip: Option<String>,
}

/// Response from `POST /v1/auth/keys`.
///
/// `raw_key` appears here exactly once, on issuance. It is never stored (only its
/// Argon2id hash and BLAKE3 lookup digest are), never logged, and never returned by
/// any other endpoint — a caller that loses it must revoke and re-issue.
#[derive(Debug, Serialize)]
pub struct IssueKeyResponse {
    pub id: uuid::Uuid,
    /// The plaintext key. Shown once; do not log this response.
    pub raw_key: String,
    pub owner: String,
    pub capabilities: Vec<String>,
    pub created_at: String,
}

/// Response from `GET /v1/auth/config`.
///
/// Reports only what is genuinely available today: whether auth is enforced and the
/// resolver type declared in configuration. Never key material, never issued tokens.
#[derive(Debug, Serialize)]
pub struct AuthConfigResponse {
    pub enabled: bool,
    pub resolver: String,
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

// ── RAG (`/v1/rag/*`) ───────────────────────────────────────────────────────

/// Body for `POST /v1/rag/collections/{name}/documents`.
///
/// Thin wrapper over `hacienda_rag`'s own IR types (`DocumentRecord`, `ChunkRecord`)
/// rather than a duplicate field-for-field DTO — those types are already
/// `Serialize`/`Deserialize` and wire-shaped, and hacienda-api adds no
/// transformation on top of them (unlike `DocumentInput`, which must base64-decode
/// and construct an `ExtractInput`).
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UpsertDocumentRequest {
    pub document: hacienda_rag::DocumentRecord,
    #[serde(default)]
    pub chunks: Vec<hacienda_rag::ChunkRecord>,
}

/// Response from `POST /v1/rag/collections/{name}/documents`.
#[derive(Debug, Serialize)]
pub struct UpsertDocumentResponse {
    pub document_id: hacienda_rag::DocumentId,
}

/// Query parameters shared by `GET /v1/rag/collections` and
/// `GET /v1/rag/collections/{name}/documents`. Each route applies its own
/// default/cap to `limit` — see `handlers/rag.rs`'s constants.
#[derive(Debug, Deserialize)]
pub struct RagListQuery {
    #[serde(default)]
    pub limit: Option<u32>,
    #[serde(default)]
    pub offset: Option<u32>,
}

/// Response from `GET /v1/rag/collections`.
///
/// `total` is the count of collections before `limit`/`offset` are applied —
/// the same total-before-pagination contract as `JobListResponse` — but
/// pushed into the backend (`RagStore::list_collections`) rather than sliced
/// in-process, since a store that can push `LIMIT`/`OFFSET` into its own
/// query never has to materialize the full set.
#[derive(Debug, Serialize)]
pub struct ListCollectionsResponse {
    pub collections: Vec<hacienda_rag::CollectionSpec>,
    pub total: u64,
}

/// Response from `GET /v1/rag/collections/{name}/documents`.
///
/// Hacienda-only addition — the real xberg-sdks OpenAPI spec has no
/// equivalent route. `DocumentSummary` is already wire-shaped and this crate
/// adds no transformation on top of it, same rationale as
/// `UpsertDocumentRequest` reusing `DocumentRecord`/`ChunkRecord` directly.
#[derive(Debug, Serialize)]
pub struct ListDocumentsResponse {
    pub documents: Vec<hacienda_rag::DocumentSummary>,
    pub total: u64,
}

/// Body for `POST /v1/rag/collections/{name}/migrate-embeddings`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MigrateEmbeddingsRequest {
    /// Target embedding preset name (e.g. `"balanced"`, `"quality"`).
    /// Validated against `xberg::get_embedding_preset` before any job is
    /// created — an unknown preset is a 400, not a job that fails later.
    pub to_source: String,
    /// The `embedding_version` to record on success. Must be strictly
    /// greater than the collection's current `embedding_version` — see
    /// `handlers::rag::migrate_embeddings`'s validation.
    pub to_version: u32,
}

/// Response from `POST /v1/rag/collections/{name}/migrate-embeddings` (202 Accepted).
#[derive(Debug, Serialize)]
pub struct MigrateEmbeddingsResponse {
    pub job_id: String,
    /// The collection's `name`, reused as the "collection id". `RagStore` has
    /// no separate UUID surrogate id — `CollectionSpec::name` is the only
    /// addressing key — unlike the real xberg-sdks spec's `format: uuid`
    /// `collection_id`. Deliberate deviation, noted here for anyone diffing
    /// against the real OpenAPI spec.
    pub collection_id: String,
    pub from_source: Option<String>,
    pub to_source: String,
    pub from_version: u32,
    pub to_version: u32,
    pub status: hacienda_core::jobs::JobStatus,
    /// Path the caller should poll for status.
    pub poll: String,
}

/// Progress payload persisted via `JobStore::update_progress` while a
/// migrate-embeddings job runs, and parsed back out of `Job::progress_json`
/// by `get_migrate_status`.
///
/// Deserialize is needed alongside Serialize for the same reason as
/// `DiffLineDto`: this type round-trips through the job store's opaque JSON
/// field rather than being serialized once for a response body.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MigrateProgressDto {
    pub documents_dual_written: u64,
    pub documents_total: u64,
    /// Always `"embedding"` in this implementation — included for shape
    /// parity with the real xberg-sdks `MigrateProgress` schema, which models
    /// a separate cutover/cleanup phase this implementation does not have
    /// (embeddings are re-computed and written in place, single phase).
    pub current_phase: String,
}

/// Response from `GET /v1/rag/collections/{name}/migrate-embeddings/{job_id}`.
#[derive(Debug, Serialize)]
pub struct MigrateStatusResponse {
    pub status: hacienda_core::jobs::JobStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub progress: Option<MigrateProgressDto>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Body for `POST /v1/rag/collections/{name}/answer` (Phase 12 Track 3).
///
/// `retrieve` and `prompt` are deliberately independent, not derived from one
/// another: `retrieve` drives what context is fetched (it may carry a
/// caller-supplied `query_vector` instead of relying on `query_text`), while
/// `prompt` is the natural-language question sent to the LLM. This mirrors
/// `hacienda_rag::stream::answer_stream`'s own `(RetrievedContext, prompt)`
/// split — see that function's doc comment.
///
/// `llm` is `xberg::LlmConfig` reused verbatim (same pattern as
/// `UpsertDocumentRequest` reusing `DocumentRecord`): it already derives
/// `Deserialize` and carries no fields this crate needs to hide or transform.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AnswerRequest {
    /// The question sent to the LLM. Redacted for PII before it leaves this
    /// process — see `handlers::rag_stream::answer`'s module doc.
    pub prompt: String,
    /// Retrieval knobs. `include_content` is forced to `true` by the handler
    /// regardless of what the caller sends: answer synthesis has nothing to
    /// ground on without chunk text.
    pub retrieve: hacienda_rag::RetrieveQuery,
    /// LLM provider/model configuration, passed to `liter-llm` verbatim.
    pub llm: xberg::LlmConfig,
    /// Optional system-prompt override — see
    /// `hacienda_rag::LlmAnswerConfig::system_prompt`.
    #[serde(default)]
    pub system_prompt: Option<String>,
}

// ── Presets (`/v1/presets/*`) ────────────────────────────────────────────────

/// Body for `POST /v1/presets`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreatePresetRequest {
    pub name: String,
    pub config: serde_json::Value,
}

/// Wire shape for a preset, shared by create/get/list responses.
#[derive(Debug, Serialize)]
pub struct PresetResponse {
    pub id: uuid::Uuid,
    pub name: String,
    pub config: serde_json::Value,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

impl From<hacienda_core::store::postgres::presets::Preset> for PresetResponse {
    fn from(preset: hacienda_core::store::postgres::presets::Preset) -> Self {
        Self {
            id: preset.id,
            name: preset.name,
            config: preset.config,
            created_at: preset.created_at,
        }
    }
}

/// Response from `GET /v1/presets`.
#[derive(Debug, Serialize)]
pub struct PresetListResponse {
    pub presets: Vec<PresetResponse>,
}

// ── Document versions (`/v1/documents/{id}/versions`, `/{id}`, `/diff*`) ──────

/// One version's summary within `GET /v1/documents/{id}/versions`.
///
/// Deliberately excludes `content`/`entities` — those are the payload of
/// `GET /v1/documents/{id}` (latest version), not a bulk listing. A document with many
/// versions should not force a client to download every redacted body just to see
/// how many versions exist and when they were created.
#[derive(Debug, Serialize)]
pub struct DocumentVersionSummaryDto {
    pub version_sequence: i64,
    pub content_hash: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

impl From<hacienda_core::store::postgres::versions::DocumentVersion> for DocumentVersionSummaryDto {
    fn from(v: hacienda_core::store::postgres::versions::DocumentVersion) -> Self {
        Self {
            version_sequence: v.version_sequence,
            content_hash: v.content_hash,
            created_at: v.created_at,
        }
    }
}

/// Response from `GET /v1/documents/{id}/versions`. Newest first, matching
/// `DocumentVersionStore::list_versions`'s documented ordering.
#[derive(Debug, Serialize)]
pub struct DocumentVersionListResponse {
    pub document_id: uuid::Uuid,
    pub versions: Vec<DocumentVersionSummaryDto>,
}

/// Response from `GET /v1/documents/{id}` — the latest version's full envelope,
/// extraction result inline (per §4.3 of the platform-parity spec).
#[derive(Debug, Serialize)]
pub struct DocumentEnvelopeResponse {
    pub document_id: uuid::Uuid,
    pub version_sequence: i64,
    pub content: String,
    pub entities: Vec<EntityDto>,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

/// Query parameters for `GET /v1/documents/{id}/diff`.
#[derive(Debug, Deserialize)]
pub struct DocumentDiffQuery {
    pub from: i64,
    pub to: i64,
}

/// A single line in a line-based diff between two versions' redacted content.
///
/// Deserialize is needed alongside Serialize: the async diff fallback round-trips
/// this through `JobStore`'s `result_json` (`GET /v1/documents/{id}/diff/{diff_job_id}`
/// parses it back out), not just serializes it once for the response body.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiffLineDto {
    /// `"equal"`, `"insert"`, or `"delete"`.
    pub op: String,
    pub text: String,
}

/// Response from `GET /v1/documents/{id}/diff` when computed synchronously (the
/// common case — most diffs are small, per §4.3).
#[derive(Debug, Serialize)]
pub struct DocumentDiffResponse {
    pub document_id: uuid::Uuid,
    pub from: i64,
    pub to: i64,
    pub lines: Vec<DiffLineDto>,
}

/// Response from `GET /v1/documents/{id}/diff` when the 2-second synchronous budget
/// is exceeded (`202 Accepted`), and from `GET /v1/documents/{id}/diff/{diff_job_id}`
/// while that job is still running.
#[derive(Debug, Serialize)]
pub struct DiffJobAcceptedResponse {
    pub diff_job_id: String,
}

/// Response from `GET /v1/documents/{id}/diff/{diff_job_id}`, polling an async diff.
///
/// Always `200` while the job exists, mirroring `GET /v1/jobs/{id}/result`: `lines` is
/// present only once `succeeded`, `error` only once `failed`.
#[derive(Debug, Serialize)]
pub struct DiffJobResultResponse {
    pub status: hacienda_core::jobs::JobStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lines: Option<Vec<DiffLineDto>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

// ── Presigned uploads (`/v1/uploads/presign`, `/v1/uploads/confirm`) ─────────

/// Request body for `POST /v1/uploads/presign`.
///
/// `filename` is metadata only (echoed nowhere yet — no route returns it back), never
/// the storage key: the key is `upload_id`, generated server-side, so a client can't
/// steer or collide with another upload's storage path via a crafted filename.
#[derive(Debug, Deserialize)]
pub struct PresignUploadRequest {
    // Accepted so the request shape documents client intent, but deliberately unread by
    // the handler: see the struct doc above.
    #[allow(dead_code)]
    pub filename: String,
    pub mime_type: String,
}

/// Response from `POST /v1/uploads/presign`.
///
/// `required_headers` must be sent verbatim by the client's `PUT` to `upload_url` — they
/// are signed into the URL, so an omitted or different header value makes the object
/// store reject the upload with a signature mismatch, not silently accept it.
#[derive(Debug, Serialize)]
pub struct PresignUploadResponse {
    pub upload_id: uuid::Uuid,
    pub upload_url: String,
    pub method: String,
    pub required_headers: std::collections::BTreeMap<String, String>,
    pub expires_at: chrono::DateTime<chrono::Utc>,
}

/// Request body for `POST /v1/uploads/confirm`.
#[derive(Debug, Deserialize)]
pub struct ConfirmUploadRequest {
    pub upload_id: uuid::Uuid,
}

/// Response from `POST /v1/uploads/confirm`.
///
/// Only returned once the object store confirms the object exists — see
/// `handlers/uploads.rs::confirm_upload`, which 404s (not 500s) when it doesn't, since an
/// unconfirmed upload is a client-triggerable condition (called before the `PUT`
/// finished, or with a stale/wrong `upload_id`), not a backend failure.
#[derive(Debug, Serialize)]
pub struct ConfirmUploadResponse {
    pub upload_id: uuid::Uuid,
    pub size_bytes: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content_type: Option<String>,
}

// ── Usage / metering (`GET /v1/usage`) ────────────────────────────────────────

/// Query parameters for `GET /v1/usage`. Both bounds are optional; an absent bound
/// leaves that side of the window open (matches `UsageStore::summary`).
#[derive(Debug, Deserialize)]
pub struct UsageQuery {
    pub since: Option<chrono::DateTime<chrono::Utc>>,
    pub until: Option<chrono::DateTime<chrono::Utc>>,
}

/// One principal's aggregate usage within the queried window.
///
/// `entity_count` and `byte_count` are the only billable units the audit chain actually
/// carries — see `hacienda_core::store::postgres::usage`'s module doc for why document
/// count is not reported here.
#[derive(Debug, Serialize)]
pub struct UsageRecordDto {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub principal: Option<String>,
    pub entity_count: i64,
    pub byte_count: i64,
}

impl From<hacienda_core::store::postgres::usage::UsageRecord> for UsageRecordDto {
    fn from(record: hacienda_core::store::postgres::usage::UsageRecord) -> Self {
        Self {
            principal: record.principal,
            entity_count: record.entity_count,
            byte_count: record.byte_count,
        }
    }
}

/// Response body for `GET /v1/usage`.
#[derive(Debug, Serialize)]
pub struct UsageResponse {
    pub records: Vec<UsageRecordDto>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub since: Option<chrono::DateTime<chrono::Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub until: Option<chrono::DateTime<chrono::Utc>>,
}
