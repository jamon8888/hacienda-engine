//! Handlers for the email processing surface (`POST /v1/email/*`).
//!
//! SECURITY (design D5): an `EmailProcessor` is NEVER shared process-wide here.
//! Each request supplies its own account/backend config and privacy mode, and a
//! fresh processor is built for that request only. The OAuth token / IMAP
//! password is resolved server-side from the environment; it is never accepted
//! in the request body, so a caller cannot point the server at an arbitrary
//! mailbox with credentials it supplied. The route's `DocumentsProcess`
//! capability gate (see `routes.rs`) is the first line of defence.

use axum::{
    extract::State,
    http::{request::Parts, StatusCode},
    Json,
};
use hacienda_email::{
    config::EmailBackend, EmailConfig, EmailDestination, EmailOperation, EmailProcessor,
    EmailRequest, EmailSource, PipelineStep, PrivacyMode,
};
use serde::{Deserialize, Serialize};
use std::time::Instant;

use crate::handlers::rag::scope_collection_name;
use crate::{
    dto::AsyncJobResponse,
    error::ApiError,
    extract::Json as SafeJson,
    handlers::{caller_from_arc, extract_auth_context},
    state::ApiState,
};

/// Body for `POST /v1/email/process`.
///
/// SECURITY: there is deliberately no `account` field and no OAuth backend
/// variant here. Both would let a caller choose which server-side secrets to
/// read (`account` → `HACIENDA_EMAIL_<ACCOUNT>_*`), which program to execute
/// (`token_command`) and which endpoint to ship the token to (`server`) — a
/// remote-code-execution plus cross-tenant credential-theft vector (CodeRabbit
/// PR #166, `backend.rs:505`). The account is instead derived from the
/// authenticated tenant, and only the local Maildir backend is reachable over
/// HTTP. Gmail/Outlook OAuth stays available on the CLI/MCP surfaces, which run
/// in-process from a trusted local config file.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EmailProcessRequest {
    /// Local Maildir path to read from. Only backend accepted over HTTP.
    pub backend: EmailBackendDto,
    /// Privacy mode for this request (per-tenant choice, design D1).
    #[serde(default)]
    pub privacy_mode: PrivacyModeDto,
    /// The operation to perform.
    pub operation: EmailOperationDto,
    /// Legal dossier (folder) the ingested emails belong to. Mirrors the
    /// `dossier_id` scoping used by document ingestion, so the same
    /// `/v1/rag/*` retrieval endpoints can scope results by dossier.
    #[serde(default)]
    pub dossier_id: Option<String>,
}

/// Only the local Maildir backend is accepted over the network. See the
/// security note on [`EmailProcessRequest`] for why OAuth is excluded.
#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum EmailBackendDto {
    Maildir { path: String },
}

#[derive(Debug, Deserialize)]
#[serde(default)]
pub struct PrivacyModeDto {
    #[serde(default)]
    pub mode: PrivacyModeValue,
}

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum PrivacyModeValue {
    #[default]
    Redact,
    Pseudonymize,
}

impl Default for PrivacyModeDto {
    fn default() -> Self {
        Self {
            mode: PrivacyModeValue::Redact,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case")]
pub enum EmailOperationDto {
    Archive {
        source: String,
        output: String,
        #[serde(default)]
        since: Option<String>,
    },
    ExtractNER {
        source: String,
        output: String,
    },
    Redact {
        source: String,
        output: String,
    },
    IndexForRag {
        source: String,
        #[serde(default)]
        index: Option<String>,
    },
    Pipeline {
        source: String,
        output: String,
    },
}

/// Response body for an email operation.
#[derive(Debug, Serialize)]
pub struct EmailProcessResponse {
    pub success: bool,
    pub processed_count: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub redaction_stats: Option<hacienda_email::models::RedactionStats>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rag_stats: Option<hacienda_email::models::RagIndexStats>,
    pub errors: Vec<String>,
    pub processing_time_ms: u64,
}

fn to_api_error(e: hacienda_email::EmailError) -> ApiError {
    ApiError::invalid_request(format!("email operation failed: {e}"))
}

/// Resolve a request-supplied Maildir path inside the authenticated tenant's
/// storage root (finding: "Confine request Maildir paths", PR #166).
///
/// The root is server-owned — `HACIENDA_MAILBOX_ROOT`, never from the request —
/// and the tenant id is appended, so one tenant cannot point at another's
/// mailbox. Both the root and the joined path are canonicalized (resolving `..`
/// and symlinks) before the containment check; anything escaping the root is
/// rejected. If the root is unset, the route is fail-closed: no path is
/// accepted until an operator configures a boundary.
async fn resolve_tenant_maildir(requested: &str, account: &str) -> Result<String, ApiError> {
    let requested = requested.to_string();
    let account = account.to_string();
    tokio::task::spawn_blocking(move || {
        let root = std::env::var_os("HACIENDA_MAILBOX_ROOT").ok_or_else(|| {
            ApiError::invalid_request(
                "email Maildir access requires the server to set HACIENDA_MAILBOX_ROOT",
            )
        })?;
        let root = std::path::Path::new(&root);
        // Reject absolute or parent-traversal request paths outright.
        let requested_path = std::path::Path::new(&requested);
        if requested_path.is_absolute()
            || requested_path
                .components()
                .any(|c| matches!(c, std::path::Component::ParentDir))
            || account.contains('/')
            || account.contains(std::path::MAIN_SEPARATOR)
        {
            return Err(ApiError::invalid_request(
                "Maildir path must be relative to the tenant's mailbox root",
            ));
        }
        let root_canonical = root.canonicalize().map_err(|e| {
            ApiError::invalid_request(format!(
                "HACIENDA_MAILBOX_ROOT is not accessible: {e}. Configure an existing directory."
            ))
        })?;
        // The tenant root is root/<account>. Containment is checked against
        // THIS root, not the global one, so a symlink inside alice's directory
        // pointing to bob's inbox is rejected.
        let tenant_root = root_canonical.join(&account);
        std::fs::create_dir_all(&tenant_root).map_err(|e| {
            ApiError::invalid_request(format!("cannot prepare tenant mailbox directory: {e}"))
        })?;
        let tenant_root_canonical = tenant_root.canonicalize().map_err(|e| {
            ApiError::invalid_request(format!("tenant mailbox root is not accessible: {e}"))
        })?;
        let candidate = tenant_root_canonical.join(requested_path);
        if let Some(parent) = candidate.parent() {
            std::fs::create_dir_all(parent).map_err(|e| {
                ApiError::invalid_request(format!("cannot prepare requested mailbox path: {e}"))
            })?;
        }
        // Reject unresolved paths: a lexical fallback would let a symlink to an
        // outside directory pass the containment check. Require a canonical
        // path that is actually inside the tenant root.
        let candidate_canonical = candidate.canonicalize().map_err(|e| {
            ApiError::invalid_request(format!(
                "Maildir path could not be resolved inside the tenant root: {e}"
            ))
        })?;
        if !candidate_canonical.starts_with(&tenant_root_canonical) {
            return Err(ApiError::invalid_request(
                "Maildir path resolves outside the tenant's mailbox root",
            ));
        }
        Ok(candidate_canonical.to_string_lossy().into_owned())
    })
    .await
    .map_err(|e| ApiError::invalid_request(format!("mailbox path resolution failed: {e}")))?
}

async fn build_config(
    body: &EmailProcessRequest,
    ctx: &hacienda_core::tenancy::TenantCtx,
) -> Result<EmailConfig, ApiError> {
    // The account comes from the authenticated tenant, never the request body.
    let account = ctx.tenant.as_str().to_string();
    // Exhaustive by construction: EmailBackendDto has exactly one variant.
    let backend = match &body.backend {
        EmailBackendDto::Maildir { path } => EmailBackend::Maildir {
            path: resolve_tenant_maildir(path, &account).await?,
        },
    };
    let mode = match body.privacy_mode.mode {
        PrivacyModeValue::Redact => PrivacyMode::Redact,
        PrivacyModeValue::Pseudonymize => PrivacyMode::Pseudonymize,
    };
    let mut config = EmailConfig {
        backend,
        // The account comes from the authenticated tenant, never the request body.
        account,
        ..Default::default()
    };
    config.redaction.mode = mode;
    // All RAG writes in this request must use the tenant-scoped collection.
    // The Pipeline variant has no index field of its own; its RagIndex step
    // falls back to this config value, so scoping here covers every path
    // (IndexForRag explicitly scopes its own index_name as well).
    config.rag.index_name = scope_collection_name(ctx, "email-corpus");
    Ok(config)
}

fn source_of(path: String) -> EmailSource {
    EmailSource::Maildir { path }
}

fn dest_of(path: String) -> EmailDestination {
    EmailDestination::Maildir { path }
}

/// Build the operation, confining every request-supplied Maildir/source/output
/// path to the authenticated tenant's root (same boundary as build_config).
async fn build_operation(
    operation: &EmailOperationDto,
    account: &str,
    tenant_ctx: &hacienda_core::tenancy::TenantCtx,
) -> Result<EmailOperation, ApiError> {
    Ok(match operation {
        EmailOperationDto::Archive {
            source,
            output,
            since,
        } => EmailOperation::Archive {
            source: source_of(resolve_tenant_maildir(source, account).await?),
            destination: dest_of(resolve_tenant_maildir(output, account).await?),
            since: since.clone(),
            before: None,
            mailbox: Some("INBOX".to_string()),
            include_attachments: true,
        },
        EmailOperationDto::ExtractNER { source, output } => EmailOperation::ExtractNER {
            source: source_of(resolve_tenant_maildir(source, account).await?),
            output: resolve_tenant_maildir(output, account).await?,
            model: None,
        },
        EmailOperationDto::Redact { source, output } => EmailOperation::Redact {
            source: source_of(resolve_tenant_maildir(source, account).await?),
            destination: dest_of(resolve_tenant_maildir(output, account).await?),
            rules: None,
        },
        EmailOperationDto::IndexForRag { source, index } => EmailOperation::IndexForRag {
            source: source_of(resolve_tenant_maildir(source, account).await?),
            // Same tenant-scoping rule as /v1/rag/* so emails indexed as
            // "email-corpus" are retrievable under "{tenant}:email-corpus".
            index_name: Some(scope_collection_name(
                tenant_ctx,
                index.as_deref().unwrap_or("email-corpus"),
            )),
            chunk_size: None,
        },
        EmailOperationDto::Pipeline { source, output } => EmailOperation::Pipeline {
            source: source_of(resolve_tenant_maildir(source, account).await?),
            destination: dest_of(resolve_tenant_maildir(output, account).await?),
            steps: vec![
                PipelineStep::Archive,
                PipelineStep::ExtractAttachments,
                PipelineStep::Ner,
                PipelineStep::RagIndex,
                PipelineStep::Redact,
            ],
        },
    })
}

/// `POST /v1/email/process/async` — the large-mailbox surface.
///
/// Same request body and per-request/per-tenant isolation as the sync route
/// (design D5), but the work runs in a background job: the caller gets `202`
/// and a `job_id` back immediately and polls `/v1/jobs/{id}` instead of holding
/// the HTTP connection for the whole ingestion.
#[utoipa::path(
    post,
    path = "/v1/email/process/async",
    tag = "email",
    operation_id = "processEmailAsync",
    security(("bearerAuth" = [])),
    responses(
        (status = 202, description = "Job accepted; poll /v1/jobs/{id}"),
        (status = 400, description = "Invalid email request"),
        (status = 401, description = "Missing or invalid credentials"),
        (status = 403, description = "Caller lacks documents:process")
    )
)]
pub async fn process_email_async(
    State(state): State<ApiState>,
    parts: Parts,
    SafeJson(body): SafeJson<EmailProcessRequest>,
) -> Result<(StatusCode, Json<AsyncJobResponse>), ApiError> {
    let ctx = extract_auth_context(&parts);
    let caller = caller_from_arc(&ctx);
    let tenant = caller.tenant_ctx().tenant;
    let owner = caller.principal_id().map(str::to_owned);

    // Validate backend AND operation before creating the job: a malformed path
    // (e.g. output="../escape") must be a 400 with no orphan job persisted.
    // The account is derived from the authenticated tenant, never the body.
    let config = build_config(&body, &caller.tenant_ctx()).await?;
    let operation = build_operation(&body.operation, tenant.as_str(), &caller.tenant_ctx()).await?;

    let job = state.jobs.create(&tenant, owner).await.map_err(|e| {
        tracing::error!(error = %e, "failed to create async email job");
        ApiError::internal()
    })?;
    let job_id = job.id.clone();

    let task_jobs = state.jobs.clone();
    let task_store = state.rag_store.clone();
    let task_dossier = body.dossier_id.clone();
    let task_tenant = tenant.clone();
    let task_job_id = job_id.clone();

    tokio::spawn(async move {
        use hacienda_core::jobs::JobStatus;

        if let Err(e) = task_jobs
            .transition(
                &task_tenant,
                &task_job_id,
                JobStatus::Queued,
                JobStatus::Running,
            )
            .await
        {
            tracing::error!(job_id = %task_job_id, error = %e, "failed to transition email job to Running");
            return;
        }

        // Progress ticks go through a single consumer so writes are serialized
        // (an unordered spawn could let an earlier page overwrite a later one).
        // The consumer is drained before the terminal finish/fail transition.
        let (progress_tx, mut progress_rx) =
            tokio::sync::mpsc::channel::<(u32, usize, usize, usize)>(64);
        let progress_jobs = task_jobs.clone();
        let progress_tenant = task_tenant.clone();
        let progress_job_id = task_job_id.clone();
        let progress_consumer = tokio::spawn(async move {
            while let Some((page, pages_done, messages, chunks)) = progress_rx.recv().await {
                let json = serde_json::json!({
                    "page": page,
                    "pages_done": pages_done,
                    "messages": messages,
                    "chunks": chunks,
                });
                if let Err(e) = progress_jobs
                    .update_progress(&progress_tenant, &progress_job_id, json.to_string())
                    .await
                {
                    tracing::warn!(job_id = %progress_job_id, error = %e, "failed to publish email job progress");
                }
            }
        });

        let progress_tx_for_cb = progress_tx.clone();
        let result = async {
            let config = config;
            let mut processor = EmailProcessor::new(config).await.map_err(to_api_error)?;
            if let Some(store) = task_store {
                processor = processor.with_rag_store(store);
            }
            if let Some(dossier) = task_dossier.as_deref() {
                processor = processor.with_dossier_id(dossier);
            }
            // `try_send` keeps the sync callback non-blocking. A full queue
            // drops a tick (progress is advisory) but never stalls ingestion.
            let on_page = move |page: u32, pages_done: usize, messages: usize, chunks: usize| {
                if progress_tx_for_cb
                    .try_send((page, pages_done, messages, chunks))
                    .is_err()
                {
                    tracing::warn!("email job progress queue full; dropping a tick");
                }
            };
            processor
                .process_with_progress(
                    EmailRequest {
                        operation,
                        config_overrides: None,
                    },
                    &on_page,
                )
                .await
                .map_err(to_api_error)
        }
        .await;

        // Close the sender and drain all queued progress before the terminal
        // state, so a late tick cannot overwrite `finish`/`fail`.
        drop(progress_tx);
        let _ = progress_consumer.await;

        match result {
            Ok(response) if response.success => match serde_json::to_string(&response) {
                Ok(json) => {
                    if let Err(e) = task_jobs.finish(&task_tenant, &task_job_id, json).await {
                        tracing::error!(
                            job_id = %task_job_id, error = %e,
                            "failed to mark email job as succeeded"
                        );
                    }
                }
                Err(e) => {
                    tracing::error!(job_id = %task_job_id, error = %e, "failed to serialize email job result");
                    let _ = task_jobs
                        .fail(
                            &task_tenant,
                            &task_job_id,
                            "serialization failed".to_string(),
                        )
                        .await;
                }
            },
            // `index_for_rag` reports per-message failures via `success=false`
            // while still returning Ok — a run where every message failed must
            // NOT be recorded as a successful job. Fail the job and keep the
            // structured errors for the operator.
            Ok(response) => {
                let reason = if response.errors.is_empty() {
                    "email processing reported failure without details".to_string()
                } else {
                    format!("email processing failed: {}", response.errors.join("; "))
                };
                tracing::error!(job_id = %task_job_id, "{}", reason);
                let _ = task_jobs.fail(&task_tenant, &task_job_id, reason).await;
            }
            Err(e) => {
                let _ = task_jobs
                    .fail(&task_tenant, &task_job_id, e.to_string())
                    .await;
            }
        }
    });

    Ok((StatusCode::ACCEPTED, Json(AsyncJobResponse { job_id })))
}

/// `POST /v1/email/process`
///
/// OpenAPI body schemas are intentionally omitted here: the request embeds
/// `hacienda_email` enum types which do not implement utoipa's `ToSchema`, and
/// adding that derive across the email crate would leak an API concern into the
/// domain layer. The route, auth requirement and status codes are still documented.
#[utoipa::path(
    post,
    path = "/v1/email/process",
    tag = "email",
    operation_id = "processEmail",
    security(("bearerAuth" = [])),
    responses(
        (status = 200, description = "Email operation result"),
        (status = 400, description = "Invalid email request"),
        (status = 401, description = "Missing or invalid credentials"),
        (status = 403, description = "Caller lacks documents:process")
    )
)]
pub async fn process_email(
    State(state): State<ApiState>,
    parts: Parts,
    SafeJson(body): SafeJson<EmailProcessRequest>,
) -> Result<Json<EmailProcessResponse>, ApiError> {
    let start = Instant::now();
    // Derive the account from the authenticated tenant (never the body).
    let ctx = extract_auth_context(&parts);
    let caller = caller_from_arc(&ctx);
    let tenant = caller.tenant_ctx().tenant;
    let config = build_config(&body, &caller.tenant_ctx()).await?;
    // Per-request processor: never shared across tenants (design D5).
    let mut processor = EmailProcessor::new(config).await.map_err(to_api_error)?;
    // Same RAG store as document ingestion: emails become searchable through
    // the existing /v1/rag/* retrieve endpoints, scoped by dossier_id.
    if let Some(store) = state.rag_store.clone() {
        processor = processor.with_rag_store(store);
    }
    if let Some(dossier) = body.dossier_id.as_deref() {
        processor = processor.with_dossier_id(dossier);
    }
    let operation = build_operation(&body.operation, tenant.as_str(), &caller.tenant_ctx()).await?;
    let response = processor
        .process(EmailRequest {
            operation,
            config_overrides: None,
        })
        .await
        .map_err(to_api_error)?;

    Ok(Json(EmailProcessResponse {
        success: response.success,
        processed_count: response.processed_count,
        redaction_stats: response.redaction_stats,
        rag_stats: response.rag_stats,
        errors: response.errors,
        processing_time_ms: start.elapsed().as_millis() as u64,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A request body must never smuggle in the OAuth/RCE fields removed after
    /// CodeRabbit PR #166 (`backend.rs:505`). `deny_unknown_fields` turns each
    /// into a hard 400 instead of silently ignoring the attempt.
    #[test]
    fn http_body_rejects_account_oauth_and_token_command_fields() {
        let op = r#""operation":{"operation":"index_for_rag","source":"/tmp/mail"}"#;
        for (field, payload) in [
            (
                "account",
                format!(
                    r#"{{"account":"victim","backend":{{"type":"maildir","path":"/tmp"}},{op}}}"#
                ),
            ),
            (
                "oauth backend",
                format!(
                    r#"{{"backend":{{"type":"oauth","username":"u","provider":"gmail","token_command":["rm","-rf","/"]}},{op}}}"#
                ),
            ),
            (
                "token_command",
                format!(
                    r#"{{"backend":{{"type":"maildir","path":"/tmp","token_command":["sh"]}},{op}}}"#
                ),
            ),
            (
                "server",
                format!(
                    r#"{{"backend":{{"type":"maildir","path":"/tmp"}},{op},"server":"evil.example"}}"#
                ),
            ),
        ] {
            let err = match serde_json::from_str::<EmailProcessRequest>(&payload) {
                Err(e) => e,
                Ok(_) => panic!("insecure payload with {field} was accepted: {payload}"),
            };
            let msg = err.to_string();
            assert!(
                msg.contains("unknown field") || msg.contains("unknown variant"),
                "payload with {field} was rejected for the wrong reason: {err}"
            );
        }
    }

    /// The minimal Maildir-only body still parses (guard against over-restricting).
    #[test]
    fn http_body_accepts_maildir_backend() {
        let payload = r#"{"backend":{"type":"maildir","path":"/tmp/mail"},"operation":{"operation":"index_for_rag","source":"/tmp/mail"}}"#;
        let request = serde_json::from_str::<EmailProcessRequest>(payload).unwrap();
        assert!(matches!(request.backend, EmailBackendDto::Maildir { .. }));
    }
}
