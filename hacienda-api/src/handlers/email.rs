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
use hacienda_core::auth::{AuthContext, Caller};
use hacienda_email::{
    config::EmailBackend, pii::PiiEngine, EmailConfig, EmailDestination, EmailOperation,
    EmailProcessor, EmailRequest, EmailSource, PipelineStep, PrivacyMode,
};
use serde::Deserialize;
use std::sync::Arc;
use std::time::Instant;

use crate::handlers::rag::scope_collection_name;
use crate::{
    dto::{AsyncJobResponse, EmailOperationResultDto},
    error::ApiError,
    extract::Json as SafeJson,
    handlers::extract_auth_context,
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

/// Serialize the transport-safe DTO that is written to `Job.result_json` by the
/// async email route and handed back by job polling.
///
/// Both routes go through [`EmailOperationResultDto::from_response`], so no
/// raw `ExtractedEntity.text` can reach a job payload or an HTTP response. This
/// helper is the exact serialization path exercised by the non-leak test below.
fn serialize_job_result(
    response: &hacienda_email::models::EmailResponse,
    processing_time_ms: u64,
) -> Result<String, serde_json::Error> {
    serde_json::to_string(&EmailOperationResultDto::from_response(
        response,
        processing_time_ms,
    ))
}

fn to_api_error(e: hacienda_email::EmailError) -> ApiError {
    // `EmailError`'s inner strings are backend diagnostics that can quote a
    // sender-controlled filename or message id. Neither the HTTP error body nor
    // the host log may carry them: emit only a fixed variant category.
    use hacienda_email::EmailError as E;
    let category = match &e {
        E::Config(_) => "config",
        E::Backend(_) => "backend",
        E::Connection(_) => "connection",
        E::Auth(_) => "auth",
        E::MailboxNotFound(_) => "mailbox_not_found",
        E::MessageNotFound(_) => "message_not_found",
        E::Io(_) => "io",
        E::Serde(_) => "serialization",
        E::UrlParse(_) => "url_parse",
        E::Ner(_) => "ner",
        E::Rag(_) => "rag",
        E::Redaction(_) => "redaction",
        E::Attachment(_) => "attachment",
        E::Plugin(_) => "plugin",
        E::InvalidOperation(_) => "invalid_operation",
    };
    tracing::error!(category, "email operation failed");
    ApiError::invalid_request(format!("email operation failed ({category})"))
}

/// The only failure text an email job is allowed to persist or log. `errors`
/// entries can carry message identifiers or backend diagnostics, so the raw
/// strings must never be joined into a client-visible or logged sentence.
fn content_free_failure_reason(error_count: usize) -> String {
    if error_count == 0 {
        "email processing reported failure without details".to_string()
    } else {
        format!("email processing failed ({error_count} content-free error entries)")
    }
}

/// Owned, tenant-scoped authenticated context for the email routes.
///
/// P0.4/P0.5: the async job cannot hold a `Caller<'_>` borrowed from the request
/// (request parts do not outlive the handler), and must never silently become
/// `Caller::Trusted` — which would act under the `default` tenant with the
/// wrong key and an unrelated audit chain. `Arc<AuthContext>` is `'static` and
/// cloneable: the spawned task owns its copy and derives a `Caller::Principal`
/// from it locally. A guarded email route without an authenticated context is
/// a server misconfiguration, so we fail closed instead of falling back.
#[derive(Clone)]
struct TenantEmailContext(Arc<AuthContext>);

impl TenantEmailContext {
    fn from_parts(parts: &Parts) -> Result<Self, ApiError> {
        match extract_auth_context(parts) {
            Some(ctx) => Ok(Self(ctx)),
            None => Err(ApiError::internal()),
        }
    }

    /// A caller borrowing this owned context. Its tenant is the authenticated
    /// tenant, never `default`; the borrow lives only as long as the task.
    fn caller(&self) -> Caller<'_> {
        Caller::Principal(self.0.as_ref())
    }

    fn tenant_ctx(&self) -> hacienda_core::tenancy::TenantCtx {
        self.caller().tenant_ctx()
    }

    fn tenant(&self) -> hacienda_core::tenancy::TenantId {
        self.tenant_ctx().tenant
    }

    fn owner(&self) -> Option<String> {
        self.caller().principal_id().map(str::to_owned)
    }
}

/// Build the PII engine from the `ApiState` facade — never from a locally
/// constructed facade, a privacy-mode enum, or an `EnvKeyResolver`.
fn host_pii(state: &ApiState) -> PiiEngine {
    PiiEngine::from_facade(state.facade.clone())
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
        // Content-free by construction: an IO error can quote a path or filename
        // chosen by the tenant. Only a fixed category reaches the client; the
        // concrete error stays on the server side (never logged with the path).
        let root_canonical = root
            .canonicalize()
            .map_err(|_| ApiError::invalid_request("Email mailbox root is not accessible."))?;
        // The tenant root is root/<account>. Containment is checked against
        // THIS root, not the global one, so a symlink inside alice's directory
        // pointing to bob's inbox is rejected.
        let tenant_root = root_canonical.join(&account);
        std::fs::create_dir_all(&tenant_root).map_err(|_| {
            ApiError::invalid_request("Cannot prepare the tenant mailbox directory.")
        })?;
        let tenant_root_canonical = tenant_root
            .canonicalize()
            .map_err(|_| ApiError::invalid_request("Tenant mailbox root is not accessible."))?;
        let candidate = tenant_root_canonical.join(requested_path);
        if let Some(parent) = candidate.parent() {
            std::fs::create_dir_all(parent).map_err(|_| {
                ApiError::invalid_request("Cannot prepare the requested mailbox path.")
            })?;
        }
        // Reject unresolved paths: a lexical fallback would let a symlink to an
        // outside directory pass the containment check. Require a canonical
        // path that is actually inside the tenant root.
        let candidate_canonical = candidate.canonicalize().map_err(|_| {
            ApiError::invalid_request("Maildir path could not be resolved inside the tenant root.")
        })?;
        if !candidate_canonical.starts_with(&tenant_root_canonical) {
            return Err(ApiError::invalid_request(
                "Maildir path resolves outside the tenant's mailbox root",
            ));
        }
        Ok(candidate_canonical.to_string_lossy().into_owned())
    })
    .await
    .map_err(|_| ApiError::invalid_request("Mailbox path resolution failed."))?
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
    let email_ctx = TenantEmailContext::from_parts(&parts)?;
    let tenant = email_ctx.tenant();
    let owner = email_ctx.owner();

    // Validate backend AND operation before creating the job: a malformed path
    // (e.g. output="../escape") must be a 400 with no orphan job persisted.
    // The account is derived from the authenticated tenant, never the body.
    let config = build_config(&body, &email_ctx.tenant_ctx()).await?;
    let operation =
        build_operation(&body.operation, tenant.as_str(), &email_ctx.tenant_ctx()).await?;

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
    // Owned, tenant-scoped and `'static`: the spawned task carries its own
    // Arc<AuthContext> instead of a Caller borrowed from `parts`, and the host
    // facade instead of building a new one (P0.4/P0.5).
    let task_ctx = email_ctx;
    let task_pii = host_pii(&state);

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

        let started = Instant::now();
        let progress_tx_for_cb = progress_tx.clone();
        let result = async {
            let config = config;
            // Derive the caller from the OWNED context inside the task. This is
            // `Caller::Principal` under the authenticated tenant — never
            // `Caller::Trusted` / the default tenant.
            let caller = task_ctx.caller();
            let mut processor = EmailProcessor::new(config, task_pii)
                .await
                .map_err(to_api_error)?;
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
                    caller,
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
            // Store only the transport-safe DTO. Serializing `response` directly
            // would persist `ExtractedEntity.text` (raw PII mentions) in
            // `Job.result_json` and expose it through job polling — the P0.2
            // leak. `serialize_job_result` is the single, tested projection.
            Ok(response) if response.success => {
                match serialize_job_result(&response, started.elapsed().as_millis() as u64) {
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
                }
            }
            // `index_for_rag` reports per-message failures via `success=false`
            // while still returning Ok — a run where every message failed must
            // NOT be recorded as a successful job. Fail the job and keep the
            // structured errors for the operator.
            Ok(response) => {
                // `response.errors` can carry message identifiers and backend
                // diagnostics. A failed job's `error` is client-visible (and logged
                // below), so persist only a bounded count and a fixed category —
                // never join the raw entries into a string that crosses a boundary.
                let error_count = response.errors.len();
                let reason = content_free_failure_reason(error_count);
                tracing::error!(job_id = %task_job_id, error_count, "email processing reported failure");
                let _ = task_jobs.fail(&task_tenant, &task_job_id, reason).await;
            }
            Err(e) => {
                // `ApiError::message` is generic by construction, but the job and
                // the log get only the machine-readable code so a future change to
                // a core error message cannot leak content through this path.
                tracing::error!(job_id = %task_job_id, code = ?e.code, "email processing failed");
                let reason = format!("email processing failed ({:?})", e.code);
                let _ = task_jobs.fail(&task_tenant, &task_job_id, reason).await;
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
) -> Result<Json<EmailOperationResultDto>, ApiError> {
    let start = Instant::now();
    // Derive the account from the authenticated tenant (never the body).
    let email_ctx = TenantEmailContext::from_parts(&parts)?;
    let caller = email_ctx.caller();
    let tenant = email_ctx.tenant();
    let config = build_config(&body, &email_ctx.tenant_ctx()).await?;
    // Per-request processor: never shared across tenants (design D5). The PII
    // authority is the ApiState facade — same tenant keys, same audit chain.
    let mut processor = EmailProcessor::new(config, host_pii(&state))
        .await
        .map_err(to_api_error)?;
    // Same RAG store as document ingestion: emails become searchable through
    // the existing /v1/rag/* retrieve endpoints, scoped by dossier_id.
    if let Some(store) = state.rag_store.clone() {
        processor = processor.with_rag_store(store);
    }
    if let Some(dossier) = body.dossier_id.as_deref() {
        processor = processor.with_dossier_id(dossier);
    }
    let operation =
        build_operation(&body.operation, tenant.as_str(), &email_ctx.tenant_ctx()).await?;
    let response = processor
        .process(
            EmailRequest {
                operation,
                config_overrides: None,
            },
            caller,
        )
        .await
        .map_err(to_api_error)?;

    // Same transport DTO as the async job path. The raw `entities` field of the
    // domain response is never serialized here (P0.2).
    Ok(Json(EmailOperationResultDto::from_response(
        &response,
        start.elapsed().as_millis() as u64,
    )))
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

    /// Failed jobs persist and log a reason string. Even when the domain layer
    /// produced PII-bearing diagnostics, only a bounded content-free sentence
    /// may cross into `Job.error` or `tracing`.
    #[test]
    fn failed_job_reason_is_content_free() {
        let reasons = [
            content_free_failure_reason(0),
            content_free_failure_reason(3),
        ];
        for reason in reasons {
            for pii in [
                "jean.dupont@example.com",
                "123-45-6789",
                "/srv/mail/inbox",
                "SENTINEL_RAW_EMAIL_ID",
            ] {
                assert!(!reason.contains(pii), "job reason leaked `{pii}`: {reason}");
            }
        }
        assert!(content_free_failure_reason(3).contains('3'));
    }

    /// A domain response whose `ExtractedEntity.text` fields contain clear-text
    /// PII must produce a job-result JSON that carries none of it (P0.2).
    /// `serialize_job_result` is the exact function the async handler uses to
    /// write `Job.result_json`, so this covers the polling surface too.
    #[test]
    fn async_job_result_never_serializes_raw_entity_text() {
        use hacienda_email::models::{EmailResponse, ExtractedEntity};

        let secrets = [
            "SENTINEL_CLEARTEXT_MENTION_JEAN_DUPONT",
            "jean.dupont@example.com",
            "+33 6 12 34 56 78",
            "SENTINEL_RAW_EMAIL_ID",
        ];
        let response = EmailResponse {
            success: true,
            processed_count: 3,
            output_path: Some("/srv/mail/redacted/INBOX/secret.mbox".to_string()),
            entities: Some(vec![
                ExtractedEntity {
                    text: secrets[0].to_string(),
                    label: "Person".to_string(),
                    confidence: 0.99,
                    email_id: secrets[3].to_string(),
                    span: (10, 41),
                },
                ExtractedEntity {
                    text: secrets[1].to_string(),
                    label: "Email".to_string(),
                    confidence: 0.98,
                    email_id: secrets[3].to_string(),
                    span: (42, 64),
                },
                ExtractedEntity {
                    text: secrets[2].to_string(),
                    label: "PhoneNumber".to_string(),
                    confidence: 0.97,
                    email_id: secrets[3].to_string(),
                    span: (65, 82),
                },
            ]),
            rag_stats: None,
            redaction_stats: None,
            // Sentinel diagnostics: envelope ids, IO/backend text, filenames and
            // PII. None of these may reach the job result or a polling response.
            errors: vec![
                "message <SENTINEL_RAW_EMAIL_ID>: io error No such file (os error 2)".to_string(),
                "failed to read /srv/mail/inbox/secret-jean-dupont.mbox".to_string(),
                "jean.dupont@example.com".to_string(),
            ],
        };

        let json = serialize_job_result(&response, 12).unwrap();

        // No raw mention, identifier, diagnostic, filename or output path may survive.
        for secret in secrets
            .iter()
            .copied()
            .chain([
                "/srv/mail/redacted/INBOX/secret.mbox",
                "SENTINEL_RAW_EMAIL_ID",
                "os error 2",
                "secret-jean-dupont.mbox",
                "jean.dupont@example.com",
            ])
        {
            assert!(
                !json.contains(secret),
                "clear-text value leaked into Job.result_json: {secret}\njson: {json}"
            );
        }

        let value: serde_json::Value = serde_json::from_str(&json).unwrap();
        let obj = value.as_object().unwrap();
        for forbidden in [
            "entities",
            "text",
            "output_path",
            "mention",
            "span",
            "email_id",
            "label",
            "errors",
        ] {
            assert!(
                !obj.contains_key(forbidden),
                "forbidden entity carrier `{forbidden}` present in job result: {json}"
            );
        }
        assert_eq!(obj["entities_found"].as_u64(), Some(3));
        assert_eq!(obj["schema_version"].as_u64(), Some(2));
        assert_eq!(obj["success"].as_bool(), Some(true));
        assert_eq!(obj["processed_count"].as_u64(), Some(3));
        assert_eq!(obj["error_count"].as_u64(), Some(3));
    }

    /// Sync API sentinel test: the exact projection used by `POST /v1/email/process`
    /// must carry no raw diagnostic. This covers the synchronous response surface;
    /// the async job surface is covered above through `serialize_job_result`.
    #[test]
    fn sync_response_errors_are_a_content_free_count() {
        use hacienda_email::models::EmailResponse;

        let sentinels = [
            "SENTINEL_ENV_550e8400-e29b-41d4-a716-446655440000",
            "io error: No such file or directory (os error 2)",
            "/srv/mail/tenant/jean-dupont.mbox",
            "jean.dupont@example.com",
        ];
        let response = EmailResponse {
            success: false,
            processed_count: 0,
            output_path: None,
            entities: None,
            rag_stats: None,
            redaction_stats: None,
            errors: sentinels.iter().map(|value| (*value).to_string()).collect(),
        };

        let dto = EmailOperationResultDto::from_response(&response, 3);
        let json = serde_json::to_string(&dto).unwrap();
        for sentinel in sentinels {
            assert!(
                !json.contains(sentinel),
                "sync response leaked `{sentinel}`: {json}"
            );
        }
        assert_eq!(dto.error_count, sentinels.len());
        assert!(!json.contains("\"errors\""));
    }

    /// The synchronous HTTP response and the asynchronous job result must be
    /// the very same DTO — not two shapes that merely both happen to redact.
    #[test]
    fn sync_and_async_routes_emit_the_same_transport_dto() {
        use hacienda_email::models::{EmailResponse, ExtractedEntity};

        let response = EmailResponse {
            success: true,
            processed_count: 1,
            output_path: Some("/srv/mail/out.mbox".to_string()),
            entities: Some(vec![ExtractedEntity {
                text: "SENTINEL_SYNC_ASYNC".to_string(),
                label: "Person".to_string(),
                confidence: 0.9,
                email_id: "msg-1".to_string(),
                span: (0, 19),
            }]),
            rag_stats: None,
            redaction_stats: None,
            errors: vec![],
        };

        // Sync route: axum serializes this value directly.
        let sync =
            serde_json::to_value(EmailOperationResultDto::from_response(&response, 7)).unwrap();
        // Async route: this is what lands in Job.result_json and is polled back.
        let async_json = serialize_job_result(&response, 7).unwrap();
        let async_value: serde_json::Value = serde_json::from_str(&async_json).unwrap();

        assert_eq!(
            sync, async_value,
            "sync HTTP response and async job result must share one DTO shape"
        );
        assert!(!async_json.contains("SENTINEL_SYNC_ASYNC"));
    }

    /// Pin the DTO's top-level contract. If a future change reintroduces an
    /// `entities`/`text` carrier, this test fails loudly instead of silently
    /// widening the confidentiality boundary.
    #[test]
    fn email_result_dto_contract_has_no_entity_carrier() {
        assert_eq!(EmailOperationResultDto::SCHEMA_VERSION, 2);
        let dto = EmailOperationResultDto {
            schema_version: 2,
            success: true,
            processed_count: 1,
            redaction_stats: None,
            rag_stats: None,
            entities_found: 0,
            error_count: 0,
            processing_time_ms: 0,
        };
        let value = serde_json::to_value(&dto).unwrap();
        let keys: std::collections::BTreeSet<&str> = value
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        let expected: std::collections::BTreeSet<&str> = [
            "schema_version",
            "success",
            "processed_count",
            "entities_found",
            "error_count",
            "processing_time_ms",
        ]
        .into_iter()
        .collect();
        assert_eq!(keys, expected);
    }

    /// P0.4/P0.5: the context an email job carries is owned (`Arc` clone) and
    /// tenant-scoped. Cloning it into a `tokio::spawn` — exactly what the async
    /// route does — must still yield `Caller::Principal` under tenant A, never
    /// `Caller::Trusted` / the default tenant.
    #[tokio::test]
    async fn async_email_context_is_owned_and_tenant_scoped() {
        use hacienda_core::auth::{Capability, CapabilitySet};
        use hacienda_core::tenancy::{TenantId, DEFAULT_TENANT};

        let ctx = TenantEmailContext(Arc::new(AuthContext::with_tenant(
            "alice",
            TenantId::new("acme"),
            CapabilitySet::new([Capability::DocumentsProcess]),
        )));
        let moved = ctx.clone();
        let tenant = tokio::spawn(async move {
            let caller = moved.caller();
            (
                caller.tenant_ctx().tenant,
                caller.principal_id().map(str::to_owned),
            )
        })
        .await
        .unwrap();

        assert_eq!(tenant.0, TenantId::new("acme"));
        assert_eq!(tenant.1.as_deref(), Some("alice"));
        assert_ne!(tenant.0, TenantId::new(DEFAULT_TENANT));
        assert_eq!(
            Caller::Trusted.tenant_ctx().tenant,
            TenantId::new(DEFAULT_TENANT),
            "a lost context would silently act under the default tenant"
        );
    }

    /// P0.4: `host_pii` must wrap the very facade the API state owns. A token
    /// minted through the email path is therefore revealable through the shared
    /// facade under the caller's tenant — no detached facade or resolver.
    #[tokio::test]
    async fn email_pii_uses_the_api_state_facade_authority() {
        use hacienda_core::auth::{Capability, CapabilitySet};
        use hacienda_core::pii::PipelineConfig;
        use hacienda_core::redaction::pseudonym::{EnvKeyResolver, ACTIVE_KEY_VAR, KEY_BYTES};
        use hacienda_core::tenancy::TenantId;
        use hacienda_core::{HaciendaConfig, HaciendaFacade};
        use std::sync::Arc as StdArc;

        let resolver = EnvKeyResolver::with_lookup(|name| match name {
            ACTIVE_KEY_VAR => Some("k1".to_string()),
            n if n.ends_with("HACIENDA_PSEUDONYM_KEY_K1") => Some("07".repeat(KEY_BYTES)),
            n if n.ends_with("HACIENDA_PSEUDONYM_ACTIVE_KEY") => Some("k1".to_string()),
            _ => None,
        });
        let config = HaciendaConfig::default().with_pii(PipelineConfig::default());
        let facade = StdArc::new(
            HaciendaFacade::with_key_resolver(config, StdArc::new(resolver), &[]).unwrap(),
        );
        let state = crate::state::ApiState::new(
            facade.clone(),
            hacienda_core::jobs::InMemoryJobStore::new().into_arc(),
            hacienda_core::auth::AuthState::new(StdArc::new(
                hacienda_core::auth::authn::DevTokenResolver,
            )),
            crate::state::ApiLimits::default(),
        );

        let ctx = AuthContext::with_tenant(
            "alice",
            TenantId::new("acme"),
            CapabilitySet::new([Capability::DocumentsProcess, Capability::PiiReveal]),
        );
        let caller = Caller::Principal(&ctx);
        let pii = host_pii(&state);
        let token = pii
            .project_metadata(caller, "subject", "jane.doe@example.com")
            .await
            .unwrap();
        assert_eq!(
            facade.reveal_token_with_auth(caller, &token).await.unwrap(),
            "jane.doe@example.com"
        );
    }
}
