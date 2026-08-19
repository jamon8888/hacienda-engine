//! Handlers for `POST /v1/documents` and `POST /v1/documents/async`.
//!
//! `ExtractInput::from_bytes` is the only `ExtractInput` constructor called here.
//! `ExtractInput::from_uri` must never appear in this crate.

use axum::{extract::State, http::request::Parts, http::StatusCode, Json};
use std::sync::Arc;
use std::time::Instant;
use xberg::ExtractInput;

use crate::{
    dto::{
        AsyncJobResponse, DocumentInput, DocumentResult, EntityDto, ProcessDocumentsRequest,
        ProcessDocumentsResponse,
    },
    error::ApiError,
    extract::Json as SafeJson,
    handlers::{caller_from_arc, extract_auth_context},
    state::ApiState,
};

/// Convert a wire `DocumentInput` to `ExtractInput::from_bytes`.
///
/// Returns an `ApiError::invalid_request` on base64 decode failure, with a message
/// that does not echo the document content.
fn to_extract_input(doc: &DocumentInput) -> Result<ExtractInput, ApiError> {
    let bytes = doc.decode_bytes().map_err(ApiError::invalid_request)?;
    Ok(ExtractInput::from_bytes(
        bytes,
        &doc.mime_type,
        doc.filename.clone(),
    ))
}

/// Validate that the batch size does not exceed the configured limit.
fn check_batch_size(count: usize, max: usize) -> Result<(), ApiError> {
    if count > max {
        return Err(ApiError::invalid_request(format!(
            "Too many documents: {count} exceeds the maximum of {max}."
        )));
    }
    Ok(())
}

/// `POST /v1/documents` — synchronous redacted extraction.
#[utoipa::path(
    post,
    path = "/v1/documents",
    tag = "documents",
    operation_id = "processDocuments",
    security(("bearerAuth" = [])),
    request_body = ProcessDocumentsRequest,
    responses(
        (status = 200, description = "Redacted extraction result for each document", body = ProcessDocumentsResponse),
        (status = 400, description = "Invalid request (bad base64, too many documents, unconfigured versioning)"),
        (status = 401, description = "Missing or invalid credentials"),
        (status = 403, description = "Caller lacks documents:process")
    )
)]
pub async fn process_documents(
    State(state): State<ApiState>,
    parts: Parts,
    SafeJson(body): SafeJson<ProcessDocumentsRequest>,
) -> Result<Json<ProcessDocumentsResponse>, ApiError> {
    let start = Instant::now();

    // Enforce document count limit before decoding content.
    check_batch_size(body.documents.len(), state.limits.max_documents)?;

    // Fail fast: a `document_id` on any input requires a configured version store,
    // checked before any extraction work runs rather than partway through the batch.
    if body.documents.iter().any(|d| d.document_id.is_some()) && state.version_store.is_none() {
        return Err(ApiError::invalid_request(
            "Document versioning is not enabled on this server.",
        ));
    }

    let ctx = extract_auth_context(&parts);
    let caller = caller_from_arc(&ctx);
    let tenant = caller.tenant_ctx().tenant;

    let inputs: Vec<ExtractInput> = body
        .documents
        .iter()
        .map(to_extract_input)
        .collect::<Result<Vec<_>, ApiError>>()?;

    let result = state
        .facade
        .process_batch_with_auth(caller, inputs)
        .await
        .map_err(ApiError::from)?;

    let audit_chain_tip = state.facade.audit_tip().await.map_err(ApiError::from)?;

    // `result.pii` is either empty (no PII pipeline configured, `HaciendaMetadata::pii_enabled`
    // is false) or exactly as long as `result.extraction.results` (one `PipelineResult` per
    // document, per `HaciendaFacade::process_batch_with_auth`'s contract) — never anything in
    // between. A three-way `.zip()` over both would silently truncate to zero when PII is
    // disabled, discarding every extracted document; iterating `pii` by `.next()` alongside the
    // extraction results and falling back to no entities once it runs dry handles both cases
    // uniformly without relying on that length invariant holding exactly.
    let mut documents: Vec<DocumentResult> = Vec::with_capacity(result.extraction.results.len());
    let mut pii_results = result.pii.into_iter();
    for (input, doc) in body.documents.iter().zip(result.extraction.results) {
        let entities = pii_results
            .next()
            .map(|pii| pii.entities)
            .unwrap_or_default();

        let (document_id, version_sequence) = if let Some(document_id) = input.document_id {
            // Checked above: version_store is Some whenever any document_id is set. Still
            // propagate rather than `.expect()` — a library handler must not panic if that
            // invariant is ever weakened.
            let store = state.version_store.as_ref().ok_or_else(|| {
                tracing::error!(
                    %document_id,
                    "version_store missing despite earlier batch-level check"
                );
                ApiError::internal()
            })?;
            let content_hash = blake3::hash(doc.content.as_bytes()).to_hex().to_string();
            // Silently falling back to `[]` here would save a version whose entities
            // disagree with the ones this response returns — a later `GET
            // /v1/documents/{id}` would then serve empty entities for a document that
            // actually had some. Fail the request instead of writing a version that
            // lies about its own content.
            let entities_json = serde_json::to_value(&entities).map_err(|error| {
                tracing::error!(
                    %error,
                    %document_id,
                    "failed to serialise entities for document version"
                );
                ApiError::internal()
            })?;
            let version_sequence = store
                .create_version(
                    &tenant,
                    document_id,
                    &content_hash,
                    &doc.content,
                    entities_json,
                )
                .await
                .map_err(ApiError::from)?;
            (Some(document_id), Some(version_sequence))
        } else {
            (None, None)
        };

        documents.push(DocumentResult {
            content: doc.content,
            entities: entities.into_iter().map(EntityDto::from).collect(),
            document_id,
            version_sequence,
        });
    }

    Ok(Json(ProcessDocumentsResponse {
        documents,
        processing_time_ms: start.elapsed().as_millis() as u64,
        audit_chain_tip,
    }))
}

/// `POST /v1/documents/async` — returns immediately with a job id.
///
/// The job runs in a detached tokio task. Jobs die with the process because the
/// store is in-memory only. A durable backend is Phase 6.
#[utoipa::path(
    post,
    path = "/v1/documents/async",
    tag = "documents",
    operation_id = "processDocumentsAsync",
    security(("bearerAuth" = [])),
    request_body = ProcessDocumentsRequest,
    responses(
        (status = 202, description = "Job accepted; poll GET /v1/jobs/{id}", body = AsyncJobResponse),
        (status = 400, description = "Invalid request"),
        (status = 401, description = "Missing or invalid credentials"),
        (status = 403, description = "Caller lacks documents:process")
    )
)]
pub async fn process_documents_async(
    State(state): State<ApiState>,
    parts: Parts,
    SafeJson(body): SafeJson<ProcessDocumentsRequest>,
) -> Result<(StatusCode, Json<AsyncJobResponse>), ApiError> {
    check_batch_size(body.documents.len(), state.limits.max_documents)?;

    let ctx = extract_auth_context(&parts);
    let caller = caller_from_arc(&ctx);
    let tenant = caller.tenant_ctx().tenant;

    // The job's owner is the principal id for per-tenant isolation.
    // `Caller::Trusted` maps to `None` (in-process caller).
    let owner = caller.principal_id().map(str::to_owned);

    let job = state.jobs.create(&tenant, owner).await.map_err(|e| {
        tracing::error!(error = %e, "failed to create async job");
        ApiError::internal()
    })?;

    let job_id = job.id.clone();

    // Decode bytes before spawning so the task does not hold the request body.
    let inputs: Vec<ExtractInput> = body
        .documents
        .iter()
        .map(to_extract_input)
        .collect::<Result<Vec<_>, ApiError>>()?;

    // Clone Arc to move into the task. The Arc<AuthContext> keeps the principal alive.
    let task_ctx: Option<Arc<hacienda_core::auth::AuthContext>> = ctx;
    let task_jobs = state.jobs.clone();
    let task_facade = state.facade.clone();
    let task_job_id = job_id.clone();
    let task_tenant = tenant.clone();

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
            tracing::error!(job_id = %task_job_id, error = %e, "failed to transition job to Running");
            return;
        }

        let result = {
            let task_caller = caller_from_arc(&task_ctx);
            task_facade
                .process_batch_with_auth(task_caller, inputs)
                .await
        };

        match result {
            Ok(res) => match serde_json::to_string(&res) {
                Ok(json) => {
                    if let Err(e) = task_jobs.finish(&task_tenant, &task_job_id, json).await {
                        tracing::error!(
                            job_id = %task_job_id, error = %e,
                            "failed to mark job as succeeded"
                        );
                    }
                }
                Err(e) => {
                    tracing::error!(
                        job_id = %task_job_id, error = %e,
                        "failed to serialise job result"
                    );
                    // Do not write the result content to the store.
                    let _ = task_jobs
                        .fail(
                            &task_tenant,
                            &task_job_id,
                            "result serialisation failed".to_string(),
                        )
                        .await;
                }
            },
            Err(e) => {
                tracing::error!(job_id = %task_job_id, error = %e, "async job failed");
                let _ = task_jobs
                    .fail(
                        &task_tenant,
                        &task_job_id,
                        "document processing failed".to_string(),
                    )
                    .await;
            }
        }
    });

    Ok((StatusCode::ACCEPTED, Json(AsyncJobResponse { job_id })))
}

#[cfg(test)]
mod tests {
    /// Assert that `ExtractInput::from_uri` never appears in this crate.
    ///
    /// `from_uri` accepts filesystem paths and HTTP URLs, giving any authenticated
    /// caller arbitrary local file reads and SSRF against the host network.
    ///
    /// This test cannot run as a `#[test]` fn — it is a compile-time / source check.
    /// We implement it as a doc string that the `no_from_uri_in_crate` integration
    /// test (in `tests/`) enforces by grepping the source.
    ///
    /// See: tests/safety.rs
    #[allow(dead_code)]
    const _SAFETY_NOTE: () = ();
}
