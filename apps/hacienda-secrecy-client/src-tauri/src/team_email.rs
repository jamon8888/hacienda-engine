//! TeamOnPremise email client for the light Tauri host.
//!
//! Every input crossing IPC here is an **opaque reference** or a bounded search
//! string. The host owns the gateway URL and the bearer credential; neither is
//! ever exposed as a Tauri command argument, so the renderer cannot steer the
//! client at another origin, present a token, or hand it an absolute Maildir
//! path, an OAuth token, a broker command, or raw message text.
//!
//! All ingestion goes through the authenticated **async** route. The job and RAG
//! routes are the same ones the backend uses for `TeamOnPremise`; the client
//! projects their responses onto content-free DTOs that are safe to serialize to
//! the renderer. Errors collapse to a small set of generic codes and never echo
//! a response body, a header, a path, or server-side detail.

use reqwest::{Client, StatusCode, Url};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::Duration;

/// Content-free error codes safe to serialize across IPC.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TeamEmailError {
    InvalidRequest,
    Unauthenticated,
    Forbidden,
    NotFound,
    RateLimited,
    Unavailable,
    InvalidResponse,
}

impl std::fmt::Display for TeamEmailError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::InvalidRequest => "team email request is invalid",
            Self::Unauthenticated => "team email authentication is required",
            Self::Forbidden => "team email access is forbidden",
            Self::NotFound => "team email resource was not found",
            Self::RateLimited => "team email service is rate limiting requests",
            Self::Unavailable => "team email service is unavailable",
            Self::InvalidResponse => "team email service returned an invalid response",
        })
    }
}

impl std::error::Error for TeamEmailError {}

type Result<T> = std::result::Result<T, TeamEmailError>;

/// Matches the backend's opaque-reference grammar. A reference cannot contain a
/// path separator, cannot begin with `.`, and carries no whitespace or shell
/// metacharacters — so it can never be widened into a path or a broker command.
fn safe_ref(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 256
        && !value.contains('/')
        && !value.contains('\\')
        && !value.starts_with('.')
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | ':'))
}

/// Pseudonym tokens carry brackets in their canonical wire form. Kept separate
/// from `safe_ref` so a plaintext address (`@`, whitespace) can never pass as a
/// `token_ref`, while a real `[EMAIL:k1:ABC]` pseudonym is accepted.
fn safe_token_ref(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 256
        && !value.contains('/')
        && !value.contains('\\')
        && !value.contains('@')
        && !value.chars().any(char::is_whitespace)
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | ':' | '[' | ']'))
}

// ── Request DTOs (used both as Tauri command arguments and HTTP bodies) ──────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case")]
pub enum TeamIngestOperation {
    IndexForRag,
    /// Canonical spelling only; the backend's legacy alias is intentionally not
    /// exposed to a new client.
    ExtractNer,
    Redact,
    Pipeline,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TeamPrivacyMode {
    #[default]
    Redact,
    Pseudonymize,
}

/// Body of `POST /v1/team/email/ingest/async`.
///
/// `deny_unknown_fields` is a hard boundary: an attempt to smuggle `backend`,
/// `path`, `token`, `password`, `server`, `token_command`, or raw message text
/// through the renderer is rejected by the Tauri command itself, before any
/// network call.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TeamIngestRequest {
    pub source_ref: String,
    pub mailbox_ref: String,
    pub operation: TeamIngestOperation,
    #[serde(default)]
    pub privacy_mode: TeamPrivacyMode,
    #[serde(default)]
    pub dossier_id: Option<String>,
}

impl TeamIngestRequest {
    fn validate(&self) -> Result<()> {
        if !safe_ref(&self.source_ref) || !safe_ref(&self.mailbox_ref) {
            return Err(TeamEmailError::InvalidRequest);
        }
        if self.dossier_id.as_deref().is_some_and(|id| !safe_ref(id)) {
            return Err(TeamEmailError::InvalidRequest);
        }
        // TeamOnPremise has no safe multi-tenant token/reveal contract. Reject
        // pseudonymization locally, before any HTTP call, matching the server.
        if self.privacy_mode == TeamPrivacyMode::Pseudonymize {
            return Err(TeamEmailError::InvalidRequest);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TeamSearchRequest {
    pub source_ref: String,
    pub mailbox_ref: String,
    pub query_text: String,
    #[serde(default)]
    pub top_k: Option<u32>,
    #[serde(default)]
    pub dossier_id: Option<String>,
}

impl TeamSearchRequest {
    fn validate(&self) -> Result<()> {
        if !safe_ref(&self.source_ref) || !safe_ref(&self.mailbox_ref) {
            return Err(TeamEmailError::InvalidRequest);
        }
        if self.dossier_id.as_deref().is_some_and(|id| !safe_ref(id)) {
            return Err(TeamEmailError::InvalidRequest);
        }
        // A user-entered query is fine; a whole message is not. Bound the query
        // so this surface cannot be used as a raw-text pipe to the server.
        let length = self.query_text.trim().len();
        if length == 0 || length > 16_000 {
            return Err(TeamEmailError::InvalidRequest);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TeamViewerCapabilityRequest {
    pub document_ref: String,
    pub email_ref: String,
    #[serde(default)]
    pub attachment_ref: Option<String>,
    /// An opaque pseudonym from a redacted projection — never a plaintext value.
    pub token_ref: String,
    #[serde(default)]
    pub purpose: Option<String>,
}

impl TeamViewerCapabilityRequest {
    fn validate(&self) -> Result<()> {
        for reference in [&self.document_ref, &self.email_ref] {
            if !safe_ref(reference) {
                return Err(TeamEmailError::InvalidRequest);
            }
        }
        if !safe_token_ref(&self.token_ref) {
            return Err(TeamEmailError::InvalidRequest);
        }
        if self
            .attachment_ref
            .as_deref()
            .is_some_and(|value| !safe_ref(value))
            || self.purpose.as_deref().is_some_and(|value| !safe_ref(value))
        {
            return Err(TeamEmailError::InvalidRequest);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TeamViewerActionRequest {
    pub capability_ref: String,
}

impl TeamViewerActionRequest {
    fn validate(&self) -> Result<()> {
        if safe_ref(&self.capability_ref) {
            Ok(())
        } else {
            Err(TeamEmailError::InvalidRequest)
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TeamJobRequest {
    pub job_id: String,
}

impl TeamJobRequest {
    pub(crate) fn validate(&self) -> Result<()> {
        if safe_ref(&self.job_id) {
            Ok(())
        } else {
            Err(TeamEmailError::InvalidRequest)
        }
    }
}

// ── Wire response DTOs (server → client) ─────────────────────────────────────────────

#[derive(Debug, Clone, Deserialize)]
pub struct WireSource {
    pub source_ref: String,
    pub mailbox_refs: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct WireSourcesResponse {
    sources: Vec<WireSource>,
}

#[derive(Debug, Deserialize)]
struct WireJobResponse {
    job_id: String,
}

#[derive(Debug, Deserialize)]
struct WireRedactionStats {
    total_redactions: usize,
    by_type: HashMap<String, usize>,
}

#[derive(Debug, Deserialize)]
struct WireRagStats {
    chunks_created: usize,
    vectors_indexed: usize,
    // Deliberately not projected to the renderer: a tenant-qualified internal
    // collection name is not a user-facing value.
    #[allow(dead_code)]
    index_name: String,
}

#[derive(Debug, Deserialize)]
struct WireOperationResult {
    schema_version: u32,
    success: bool,
    processed_count: usize,
    #[serde(default)]
    redaction_stats: Option<WireRedactionStats>,
    #[serde(default)]
    rag_stats: Option<WireRagStats>,
    entities_found: usize,
    // The server emits only a content-free count. Accepting `Vec<String>` here
    // would let a future server regression smuggle raw diagnostics to the IPC
    // boundary undetected; a numeric count cannot carry PII.
    error_count: usize,
    processing_time_ms: u64,
}

#[derive(Debug, Deserialize)]
struct WireJobStatus {
    id: String,
    status: String,
    #[serde(default)]
    result: Option<serde_json::Value>,
    #[serde(default)]
    progress: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
struct WireJobResult {
    status: String,
    #[serde(default)]
    result: Option<serde_json::Value>,
    #[serde(default)]
    progress: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
struct WireSearchHit {
    document_ref: String,
    external_ref: String,
    source_kind: String,
    mime: Option<String>,
    dossier_id: Option<String>,
    version_ref: Option<String>,
    ordinal: u32,
    score: f32,
    projection: String,
    viewer_policy: String,
}

#[derive(Debug, Deserialize)]
struct WireSearchResponse {
    hits: Vec<WireSearchHit>,
}

#[derive(Debug, Deserialize)]
struct WireViewerCapability {
    capability_ref: String,
    expires_at: String,
    document_ref: String,
    email_ref: String,
    attachment_ref: Option<String>,
    delivery: String,
}

#[derive(Debug, Deserialize)]
struct WireViewerReveal {
    capability_ref: String,
    revealed: bool,
    document_ref: String,
    email_ref: String,
}

#[derive(Debug, Deserialize)]
struct WireViewerHeader {
    name: String,
    token_only: String,
}

#[derive(Debug, Deserialize)]
struct WireViewerProjection {
    capability_ref: String,
    document_ref: String,
    email_ref: String,
    headers: Vec<WireViewerHeader>,
    body_token_only: String,
    attachment_refs: Vec<String>,
    projection_kind: String,
}

#[derive(Debug, Deserialize)]
struct WireAuditEvent {
    sequence: u64,
    timestamp: String,
    action: String,
    tenant: String,
    principal: String,
    document_ref: String,
    email_ref: String,
    attachment_ref: Option<String>,
    capability_ref: String,
    outcome: String,
}

#[derive(Debug, Deserialize)]
struct WireAuditResponse {
    events: Vec<WireAuditEvent>,
}

// ── Renderer-safe response DTOs (client → Tauri command → renderer) ──────────────────

#[derive(Debug, Clone, Serialize)]
pub struct TeamSourceDto {
    pub source_ref: String,
    pub mailbox_refs: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct TeamSourcesResponse {
    pub sources: Vec<TeamSourceDto>,
}

#[derive(Debug, Clone, Serialize)]
pub struct TeamAcceptedJob {
    pub job_id: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct TeamOperationResultDto {
    pub schema_version: u32,
    pub success: bool,
    pub processed_count: usize,
    pub redaction_count: usize,
    pub redaction_by_type: HashMap<String, usize>,
    pub chunks_created: usize,
    pub vectors_indexed: usize,
    pub entities_found: usize,
    pub error_count: usize,
    pub processing_time_ms: u64,
}

impl From<WireOperationResult> for TeamOperationResultDto {
    fn from(wire: WireOperationResult) -> Self {
        // `index_name`, raw spans and paths are dropped here. `error_count` is a
        // numeric, content-free count supplied by the server. This is the single
        // projection from a server payload onto a renderer DTO.
        Self {
            schema_version: wire.schema_version,
            success: wire.success,
            processed_count: wire.processed_count,
            redaction_count: wire
                .redaction_stats
                .as_ref()
                .map_or(0, |stats| stats.total_redactions),
            redaction_by_type: wire
                .redaction_stats
                .map(|stats| stats.by_type)
                .unwrap_or_default(),
            chunks_created: wire.rag_stats.as_ref().map_or(0, |s| s.chunks_created),
            vectors_indexed: wire.rag_stats.as_ref().map_or(0, |s| s.vectors_indexed),
            entities_found: wire.entities_found,
            error_count: wire.error_count,
            processing_time_ms: wire.processing_time_ms,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct TeamProgressDto {
    pub page: u32,
    pub pages_done: usize,
    pub messages: usize,
    pub chunks: usize,
}

fn project_progress(value: Option<serde_json::Value>) -> Option<TeamProgressDto> {
    let value = value?;
    Some(TeamProgressDto {
        page: value.get("page")?.as_u64()? as u32,
        pages_done: value.get("pages_done")?.as_u64()? as usize,
        messages: value.get("messages")?.as_u64()? as usize,
        chunks: value.get("chunks")?.as_u64()? as usize,
    })
}

fn project_result(value: Option<serde_json::Value>) -> Option<TeamOperationResultDto> {
    serde_json::from_value::<WireOperationResult>(value?).ok().map(Into::into)
}

#[derive(Debug, Clone, Serialize)]
pub struct TeamJobStatusDto {
    pub job_id: String,
    pub status: String,
    /// `true` only for the terminal failure state; no failure text is exposed.
    pub failed: bool,
    pub progress: Option<TeamProgressDto>,
    pub result: Option<TeamOperationResultDto>,
}

impl TeamJobStatusDto {
    fn from_wire(wire: WireJobStatus) -> Self {
        Self {
            job_id: wire.id,
            failed: wire.status == "failed",
            status: wire.status,
            progress: project_progress(wire.progress),
            result: project_result(wire.result),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct TeamSearchHitDto {
    pub document_ref: String,
    pub external_ref: String,
    pub source_kind: String,
    pub mime: Option<String>,
    pub dossier_id: Option<String>,
    pub version_ref: Option<String>,
    pub ordinal: u32,
    pub score: f32,
    pub projection: String,
    pub viewer_policy: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct TeamSearchResponse {
    pub hits: Vec<TeamSearchHitDto>,
}

#[derive(Debug, Clone, Serialize)]
pub struct TeamViewerCapabilityDto {
    pub capability_ref: String,
    pub expires_at: String,
    pub document_ref: String,
    pub email_ref: String,
    pub attachment_ref: Option<String>,
    pub delivery: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct TeamViewerRevealResultDto {
    pub capability_ref: String,
    pub revealed: bool,
    pub document_ref: String,
    pub email_ref: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct TeamViewerHeaderDto {
    pub name: String,
    pub token_only: String,
}

/// Renderer-safe viewer projection. Headers and body are already redacted /
/// token-only on the server; attachment fields are opaque references. There is
/// no `opened` boolean and no clear-text field on this DTO.
#[derive(Debug, Clone, Serialize)]
pub struct TeamViewerProjectionDto {
    pub capability_ref: String,
    pub document_ref: String,
    pub email_ref: String,
    pub headers: Vec<TeamViewerHeaderDto>,
    pub body_token_only: String,
    pub attachment_refs: Vec<String>,
    pub projection_kind: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct TeamAuditEventDto {
    pub sequence: u64,
    pub timestamp: String,
    pub action: String,
    pub tenant: String,
    pub principal: String,
    pub document_ref: String,
    pub email_ref: String,
    pub attachment_ref: Option<String>,
    pub capability_ref: String,
    pub outcome: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct TeamAuditResponse {
    pub events: Vec<TeamAuditEventDto>,
}

// ── The authenticated client ─────────────────────────────────────────────────────────

/// An authenticated client bound, at construction, to one on-premise gateway.
/// Both the base URL and the bearer token are host-owned values; no command can
/// change either one.
#[derive(Debug, Clone)]
pub struct TeamEmailClient {
    http: Client,
    base: Url,
    token: String,
}

impl TeamEmailClient {
    pub fn new(base_url: impl Into<String>, token: impl Into<String>) -> Result<Self> {
        let base_url = base_url.into();
        let token = token.into();
        if token.is_empty() {
            return Err(TeamEmailError::InvalidRequest);
        }
        let base = Url::parse(&base_url).map_err(|_| TeamEmailError::InvalidRequest)?;
        if !matches!(base.scheme(), "http" | "https")
            || !base.username().is_empty()
            || base.password().is_some()
            || base.query().is_some()
            || base.fragment().is_some()
        {
            return Err(TeamEmailError::InvalidRequest);
        }
        let http = Client::builder()
            .timeout(Duration::from_secs(60))
            .build()
            .map_err(|_| TeamEmailError::Unavailable)?;
        Ok(Self { http, base, token })
    }

    fn url(&self, path: &str) -> Result<Url> {
        self.base.join(path).map_err(|_| TeamEmailError::InvalidRequest)
    }

    async fn send(&self, request: reqwest::RequestBuilder) -> Result<reqwest::Response> {
        let response = request
            .bearer_auth(&self.token)
            .send()
            .await
            .map_err(|_| TeamEmailError::Unavailable)?;
        let status = response.status();
        if !status.is_success() {
            // Read and discard the body: an upstream error may contain message
            // content. Only the generic status code crosses this boundary.
            let _ = response.bytes().await;
            return Err(map_status(status));
        }
        Ok(response)
    }

    async fn get_json<T: for<'de> Deserialize<'de>>(&self, path: &str) -> Result<T> {
        let request = self.http.get(self.url(path)?);
        let response = self.send(request).await?;
        response.json::<T>().await.map_err(|_| TeamEmailError::InvalidResponse)
    }

    async fn post_json<B: Serialize, T: for<'de> Deserialize<'de>>(
        &self,
        path: &str,
        body: &B,
    ) -> Result<T> {
        let request = self.http.post(self.url(path)?).json(body);
        let response = self.send(request).await?;
        response.json::<T>().await.map_err(|_| TeamEmailError::InvalidResponse)
    }

    pub async fn sources(&self) -> Result<TeamSourcesResponse> {
        let wire = self
            .get_json::<WireSourcesResponse>("/v1/team/email/sources")
            .await?;
        Ok(TeamSourcesResponse {
            sources: wire
                .sources
                .into_iter()
                .map(|source| TeamSourceDto {
                    source_ref: source.source_ref,
                    mailbox_refs: source.mailbox_refs,
                })
                .collect(),
        })
    }

    /// Queue an ingestion job. The body contains only server-provisioned refs.
    pub async fn ingest_async(&self, request: &TeamIngestRequest) -> Result<TeamAcceptedJob> {
        request.validate()?;
        let wire = self
            .post_json::<_, WireJobResponse>("/v1/team/email/ingest/async", request)
            .await?;
        if !safe_ref(&wire.job_id) {
            return Err(TeamEmailError::InvalidResponse);
        }
        Ok(TeamAcceptedJob { job_id: wire.job_id })
    }

    pub async fn job(&self, job_id: &str) -> Result<TeamJobStatusDto> {
        if !safe_ref(job_id) {
            return Err(TeamEmailError::InvalidRequest);
        }
        let path = format!("/v1/jobs/{job_id}");
        let wire = self.get_json::<WireJobStatus>(&path).await?;
        Ok(TeamJobStatusDto::from_wire(wire))
    }

    /// Fetch the terminal payload from the dedicated result route. The status
    /// poll above carries only counters; the result endpoint carries the single
    /// transport DTO that both sync and async ingestion produce.
    pub async fn job_result(&self, job_id: &str) -> Result<TeamJobStatusDto> {
        if !safe_ref(job_id) {
            return Err(TeamEmailError::InvalidRequest);
        }
        let path = format!("/v1/jobs/{job_id}/result");
        let wire = self.get_json::<WireJobResult>(&path).await?;
        Ok(TeamJobStatusDto {
            job_id: job_id.to_string(),
            failed: wire.status == "failed",
            status: wire.status,
            progress: project_progress(wire.progress),
            result: project_result(wire.result),
        })
    }

    /// Poll until the job reaches a terminal state or the attempt budget is spent.
    /// The returned DTO is identical in shape whether the work came from a sync
    /// or an async ingestion, because both are projected by `project_result`.
    pub async fn wait_for_job(
        &self,
        job_id: &str,
        max_attempts: u32,
        delay: Duration,
    ) -> Result<TeamJobStatusDto> {
        for _ in 0..max_attempts.max(1) {
            let status = self.job(job_id).await?;
            if matches!(status.status.as_str(), "succeeded" | "failed") {
                return Ok(self.job_result(job_id).await?);
            }
            tokio::time::sleep(delay).await;
        }
        Ok(self.job_result(job_id).await?)
    }

    pub async fn search(&self, request: &TeamSearchRequest) -> Result<TeamSearchResponse> {
        request.validate()?;
        let wire = self
            .post_json::<_, WireSearchResponse>("/v1/team/email/search", request)
            .await?;
        Ok(TeamSearchResponse {
            hits: wire
                .hits
                .into_iter()
                .map(|hit| TeamSearchHitDto {
                    document_ref: hit.document_ref,
                    external_ref: hit.external_ref,
                    source_kind: hit.source_kind,
                    mime: hit.mime,
                    dossier_id: hit.dossier_id,
                    version_ref: hit.version_ref,
                    ordinal: hit.ordinal,
                    score: hit.score,
                    projection: hit.projection,
                    viewer_policy: hit.viewer_policy,
                })
                .collect(),
        })
    }

    pub async fn issue_capability(
        &self,
        request: &TeamViewerCapabilityRequest,
    ) -> Result<TeamViewerCapabilityDto> {
        request.validate()?;
        let wire = self
            .post_json::<_, WireViewerCapability>("/v1/team/viewer/capability", request)
            .await?;
        Ok(TeamViewerCapabilityDto {
            capability_ref: wire.capability_ref,
            expires_at: wire.expires_at,
            document_ref: wire.document_ref,
            email_ref: wire.email_ref,
            attachment_ref: wire.attachment_ref,
            delivery: wire.delivery,
        })
    }

    pub async fn reveal(
        &self,
        request: &TeamViewerActionRequest,
    ) -> Result<TeamViewerRevealResultDto> {
        request.validate()?;
        let wire = self
            .post_json::<_, WireViewerReveal>("/v1/team/viewer/reveal", request)
            .await?;
        Ok(TeamViewerRevealResultDto {
            capability_ref: wire.capability_ref,
            revealed: wire.revealed,
            document_ref: wire.document_ref,
            email_ref: wire.email_ref,
        })
    }

    pub async fn open(&self, request: &TeamViewerActionRequest) -> Result<TeamViewerProjectionDto> {
        request.validate()?;
        let wire = self
            .post_json::<_, WireViewerProjection>("/v1/team/viewer/open", request)
            .await?;
        Ok(TeamViewerProjectionDto {
            capability_ref: wire.capability_ref,
            document_ref: wire.document_ref,
            email_ref: wire.email_ref,
            headers: wire
                .headers
                .into_iter()
                .map(|header| TeamViewerHeaderDto {
                    name: header.name,
                    token_only: header.token_only,
                })
                .collect(),
            body_token_only: wire.body_token_only,
            attachment_refs: wire.attachment_refs,
            projection_kind: wire.projection_kind,
        })
    }

    pub async fn audit(&self) -> Result<TeamAuditResponse> {
        let wire = self.get_json::<WireAuditResponse>("/v1/team/viewer/audit").await?;
        Ok(TeamAuditResponse {
            events: wire
                .events
                .into_iter()
                .map(|event| TeamAuditEventDto {
                    sequence: event.sequence,
                    timestamp: event.timestamp,
                    action: event.action,
                    tenant: event.tenant,
                    principal: event.principal,
                    document_ref: event.document_ref,
                    email_ref: event.email_ref,
                    attachment_ref: event.attachment_ref,
                    capability_ref: event.capability_ref,
                    outcome: event.outcome,
                })
                .collect(),
        })
    }
}

fn map_status(status: StatusCode) -> TeamEmailError {
    match status.as_u16() {
        400 | 413 | 415 => TeamEmailError::InvalidRequest,
        401 => TeamEmailError::Unauthenticated,
        403 => TeamEmailError::Forbidden,
        404 => TeamEmailError::NotFound,
        429 => TeamEmailError::RateLimited,
        _ => TeamEmailError::Unavailable,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ingest_dto_rejects_paths_credentials_and_raw_text() {
        let base = r#"{"source_ref":"default","mailbox_ref":"inbox","operation":{"operation":"index_for_rag"}}"#;
        assert!(serde_json::from_str::<TeamIngestRequest>(base).is_ok());
        for (field, body) in [
            ("backend", base.replace('}', r#","backend":{"path":"/etc"}}"#)),
            ("path", base.replace('}', r#","path":"/etc"}"#)),
            ("token", base.replace('}', r#","token":"oauth-secret"}"#)),
            ("password", base.replace('}', r#","password":"hunter2"}"#)),
            ("server", base.replace('}', r#","server":"evil.example"}"#)),
            ("token_command", base.replace('}', r#","token_command":["sh"]}"#)),
            ("text", base.replace('}', r#","text":"raw message"}"#)),
            ("message", base.replace('}', r#","message":"raw message"}"#)),
        ] {
            assert!(
                serde_json::from_str::<TeamIngestRequest>(&body).is_err(),
                "insecure field `{field}` was accepted"
            );
        }
    }

    #[test]
    fn ingest_dto_validates_opaque_refs_only() {
        let good: TeamIngestRequest = serde_json::from_str(
            r#"{"source_ref":"src-1","mailbox_ref":"inbox","operation":{"operation":"extract_ner"}}"#,
        )
        .unwrap();
        assert!(good.validate().is_ok());
        for (source, mailbox) in [
            ("../etc", "inbox"),
            ("src", "/absolute/path"),
            // JSON needs the backslash escaped; the Rust literal must carry two.
            ("src", "..\\\\victim"),
            ("default", ""),
        ] {
            let request: TeamIngestRequest = serde_json::from_str(&format!(
                r#"{{"source_ref":"{source}","mailbox_ref":"{mailbox}","operation":{{"operation":"index_for_rag"}}}}"#
            ))
            .unwrap();
            assert_eq!(request.validate(), Err(TeamEmailError::InvalidRequest));
        }
    }

    #[test]
    fn team_ingest_rejects_pseudonymize_before_any_http_call() {
        let request: TeamIngestRequest = serde_json::from_str(
            r#"{"source_ref":"default","mailbox_ref":"inbox","operation":{"operation":"index_for_rag"},"privacy_mode":"pseudonymize"}"#,
        )
        .unwrap();
        assert_eq!(request.validate(), Err(TeamEmailError::InvalidRequest));
    }

    #[test]
    fn viewer_dto_rejects_plaintext_and_path_carriers() {
        let base = r#"{"document_ref":"doc1","email_ref":"msg1","token_ref":"tok1"}"#;
        assert!(serde_json::from_str::<TeamViewerCapabilityRequest>(base).is_ok());
        for body in [
            base.replace('}', r#","value":"jean.dupont@example.com"}"#),
            base.replace('}', r#","path":"/srv/mail/inbox"}"#),
            base.replace('}', r#","password":"secret"}"#),
            base.replace('}', r#","raw_text":"raw message"}"#),
        ] {
            assert!(
                serde_json::from_str::<TeamViewerCapabilityRequest>(&body).is_err(),
                "insecure viewer body accepted: {body}"
            );
        }
    }

    #[test]
    fn renderer_result_dtos_have_no_raw_content_carrier() {
        let result: WireOperationResult = serde_json::from_str(
            r#"{"schema_version":1,"success":true,"processed_count":2,"entities_found":1,
                "error_count":1,"processing_time_ms":5,
                "rag_stats":{"chunks_created":3,"vectors_indexed":3,"index_name":"acme:email-corpus"}}"#,
        )
        .unwrap();
        let dto: TeamOperationResultDto = result.into();
        let json = serde_json::to_value(&dto).unwrap();
        let text = json.to_string();
        for pii in [
            "jean.dupont@example.com",
            "acme:email-corpus",
            "errors",
            "index_name",
            "path",
            "plaintext",
        ] {
            assert!(!text.contains(pii), "renderer DTO leaked `{pii}`: {text}");
        }
        assert_eq!(dto.error_count, 1);
    }

    #[test]
    fn errors_are_content_free_even_when_the_server_echoes_pii() {
        for error in [
            TeamEmailError::InvalidRequest,
            TeamEmailError::Unauthenticated,
            TeamEmailError::Forbidden,
            TeamEmailError::NotFound,
            TeamEmailError::RateLimited,
            TeamEmailError::Unavailable,
            TeamEmailError::InvalidResponse,
        ] {
            let rendered = format!("{error}") + &serde_json::to_string(&error).unwrap();
            for pii in ["jean.dupont@example.com", "/srv/mail", "oauth", "/etc"] {
                assert!(!rendered.contains(pii), "error leaked `{pii}`: {rendered}");
            }
        }
    }

    #[test]
    fn client_rejects_credential_bearing_base_urls() {
        for url in [
            "https://user:pass@gateway.example",
            "file:///etc/passwd",
            "https://gateway.example/?token=secret",
            "not-a-url",
        ] {
            assert!(
                TeamEmailClient::new(url, "token").is_err(),
                "unsafe base url accepted: {url}"
            );
        }
        assert!(TeamEmailClient::new("https://gateway.example", "token").is_ok());
        assert!(TeamEmailClient::new("https://gateway.example", "").is_err());
    }

    #[test]
    fn job_id_is_validated_before_it_can_reach_a_path() {
        for job_id in ["../other-tenant", "a/b", "..", "", "/etc/passwd"] {
            assert!(TeamJobRequest { job_id: job_id.into() }.validate().is_err());
        }
        assert!(TeamJobRequest { job_id: "job-123".into() }.validate().is_ok());
    }
}
