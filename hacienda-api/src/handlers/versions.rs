//! Handlers for `GET /v1/documents/{id}/versions`, `GET /v1/documents/{id}`, and the
//! `/diff`/`/diff/{diff_job_id}` pair.
//!
//! `DocumentVersionStore` has no in-memory backend (Postgres-only) — like `PresetStore`,
//! these routes 400 when `ApiState::version_store` is `None`. See `handlers/presets.rs`
//! for the identical opt-in pattern this mirrors.

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use hacienda_core::jobs::JobStatus;
use hacienda_core::store::postgres::versions::DocumentVersionStore;
use serde_json::Value;
use std::sync::Arc;
use std::time::Duration;
use uuid::Uuid;

use crate::{
    dto::{
        DiffJobAcceptedResponse, DiffJobResultResponse, DiffLineDto, DocumentDiffQuery,
        DocumentDiffResponse, DocumentEnvelopeResponse, DocumentVersionListResponse,
        DocumentVersionSummaryDto, EntityDto,
    },
    error::ApiError,
    extract::Query,
    state::ApiState,
};

/// Wall-clock budget a synchronous `GET /v1/documents/{id}/diff` gets before falling
/// back to `202 Accepted` + `diff_job_id`, per §4.3 of the platform-parity spec.
const DIFF_SYNC_BUDGET: Duration = Duration::from_secs(2);

fn require_store(state: &ApiState) -> Result<&Arc<dyn DocumentVersionStore>, ApiError> {
    state.version_store.as_ref().ok_or_else(|| {
        ApiError::invalid_request("Document versioning is not enabled on this server.")
    })
}

/// Deserialize a version's stored `entities_json` back into wire `EntityDto`s.
///
/// Stored as `Vec<hacienda_core::pii::MergedEntity>` JSON (see `handlers/documents.rs`).
/// A malformed value (only possible via direct DB tampering, never through this API)
/// degrades to an empty list rather than 500ing the whole response.
fn entities_from_json(entities_json: &Value) -> Vec<EntityDto> {
    serde_json::from_value::<Vec<hacienda_core::pii::MergedEntity>>(entities_json.clone())
        .unwrap_or_default()
        .into_iter()
        .map(EntityDto::from)
        .collect()
}

/// `GET /v1/documents/{id}/versions` — newest first.
#[utoipa::path(
    get,
    path = "/v1/documents/{id}/versions",
    tag = "versions",
    operation_id = "listDocumentVersions",
    security(("bearerAuth" = [])),
    params(("id" = Uuid, Path, description = "Client-supplied document id")),
    responses(
        (status = 200, description = "Versions newest-first", body = DocumentVersionListResponse),
        (status = 400, description = "Document versioning is not enabled on this server")
    )
)]
pub async fn list_document_versions(
    State(state): State<ApiState>,
    Path(document_id): Path<Uuid>,
) -> Result<Json<DocumentVersionListResponse>, ApiError> {
    let store = require_store(&state)?;
    let versions = store
        .list_versions(document_id)
        .await
        .map_err(ApiError::from)?;

    Ok(Json(DocumentVersionListResponse {
        document_id,
        versions: versions
            .into_iter()
            .map(DocumentVersionSummaryDto::from)
            .collect(),
    }))
}

/// `GET /v1/documents/{id}` — latest version's full envelope, extraction result inline.
#[utoipa::path(
    get,
    path = "/v1/documents/{id}",
    tag = "versions",
    operation_id = "getDocument",
    security(("bearerAuth" = [])),
    params(("id" = Uuid, Path, description = "Client-supplied document id")),
    responses(
        (status = 200, description = "Latest version, extraction result inline", body = DocumentEnvelopeResponse),
        (status = 400, description = "Document versioning is not enabled on this server"),
        (status = 404, description = "No version exists for this document id")
    )
)]
pub async fn get_document(
    State(state): State<ApiState>,
    Path(document_id): Path<Uuid>,
) -> Result<Json<DocumentEnvelopeResponse>, ApiError> {
    let store = require_store(&state)?;
    let versions = store
        .list_versions(document_id)
        .await
        .map_err(ApiError::from)?;
    let latest = versions
        .into_iter()
        .next()
        .ok_or_else(ApiError::not_found)?;

    Ok(Json(DocumentEnvelopeResponse {
        document_id,
        version_sequence: latest.version_sequence,
        entities: entities_from_json(&latest.entities_json),
        content: latest.content,
        created_at: latest.created_at,
    }))
}

/// Hand-written line-based diff (LCS over `str::lines()`). No diff crate exists in the
/// workspace, and the plan calls for exactly this: "a byte-level diff is enough, no need
/// for a semantic diff library." CPU cost is O(n*m) in line counts; this is why the
/// caller runs it via `spawn_blocking` under a wall-clock budget rather than inline.
fn compute_line_diff(from: &str, to: &str) -> Vec<DiffLineDto> {
    let from_lines: Vec<&str> = from.lines().collect();
    let to_lines: Vec<&str> = to.lines().collect();
    let n = from_lines.len();
    let m = to_lines.len();

    let mut lcs = vec![vec![0u32; m + 1]; n + 1];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            lcs[i][j] = if from_lines[i] == to_lines[j] {
                lcs[i + 1][j + 1] + 1
            } else {
                lcs[i + 1][j].max(lcs[i][j + 1])
            };
        }
    }

    let mut lines = Vec::new();
    let (mut i, mut j) = (0, 0);
    while i < n && j < m {
        if from_lines[i] == to_lines[j] {
            lines.push(DiffLineDto {
                op: "equal".to_string(),
                text: from_lines[i].to_string(),
            });
            i += 1;
            j += 1;
        } else if lcs[i + 1][j] >= lcs[i][j + 1] {
            lines.push(DiffLineDto {
                op: "delete".to_string(),
                text: from_lines[i].to_string(),
            });
            i += 1;
        } else {
            lines.push(DiffLineDto {
                op: "insert".to_string(),
                text: to_lines[j].to_string(),
            });
            j += 1;
        }
    }
    for line in &from_lines[i..n] {
        lines.push(DiffLineDto {
            op: "delete".to_string(),
            text: (*line).to_string(),
        });
    }
    for line in &to_lines[j..m] {
        lines.push(DiffLineDto {
            op: "insert".to_string(),
            text: (*line).to_string(),
        });
    }
    lines
}

/// `GET /v1/documents/{id}/diff?from=&to=` — synchronous by default, 2-second budget.
///
/// Over budget, hands off to a detached task that finishes the computation and records
/// it via `JobStore`, returning `202 Accepted` + `diff_job_id` for the caller to poll
/// at `GET /v1/documents/{id}/diff/{diff_job_id}` — reusing `JobStore` rather than
/// inventing a second async mechanism, per §4.3.
#[utoipa::path(
    get,
    path = "/v1/documents/{id}/diff",
    tag = "versions",
    operation_id = "diffDocument",
    security(("bearerAuth" = [])),
    params(
        ("id" = Uuid, Path, description = "Client-supplied document id"),
        ("from" = i64, Query, description = "Source version_sequence"),
        ("to" = i64, Query, description = "Target version_sequence")
    ),
    responses(
        (status = 200, description = "Line diff, computed synchronously", body = DocumentDiffResponse),
        (status = 202, description = "Over the 2s sync budget; poll diff_job_id", body = DiffJobAcceptedResponse),
        (status = 400, description = "Document versioning is not enabled on this server"),
        (status = 404, description = "Unknown from/to version")
    )
)]
pub async fn diff_document(
    State(state): State<ApiState>,
    Path(document_id): Path<Uuid>,
    Query(query): Query<DocumentDiffQuery>,
) -> Result<Response, ApiError> {
    let store = require_store(&state)?;

    let from = store
        .get_version(document_id, query.from)
        .await
        .map_err(ApiError::from)?
        .ok_or_else(ApiError::not_found)?;
    let to = store
        .get_version(document_id, query.to)
        .await
        .map_err(ApiError::from)?
        .ok_or_else(ApiError::not_found)?;

    let job = state.jobs.create(None).await.map_err(|e| {
        tracing::error!(error = %e, "failed to create diff job");
        ApiError::internal()
    })?;
    let job_id = job.id.clone();
    state
        .jobs
        .transition(&job_id, JobStatus::Queued, JobStatus::Running)
        .await
        .map_err(|e| {
            tracing::error!(job_id = %job_id, error = %e, "failed to transition diff job to running");
            ApiError::internal()
        })?;

    let mut handle =
        tokio::task::spawn_blocking(move || compute_line_diff(&from.content, &to.content));

    tokio::select! {
        result = &mut handle => {
            match result {
                Ok(lines) => {
                    let json = serde_json::to_string(&lines).unwrap_or_else(|_| "[]".to_string());
                    let _ = state.jobs.finish(&job_id, json).await;
                    Ok(Json(DocumentDiffResponse {
                        document_id,
                        from: query.from,
                        to: query.to,
                        lines,
                    })
                    .into_response())
                }
                Err(join_err) => {
                    tracing::error!(job_id = %job_id, error = %join_err, "diff computation panicked");
                    let _ = state
                        .jobs
                        .fail(&job_id, "diff computation failed".to_string())
                        .await;
                    Err(ApiError::internal())
                }
            }
        }
        () = tokio::time::sleep(DIFF_SYNC_BUDGET) => {
            let task_jobs = state.jobs.clone();
            let task_job_id = job_id.clone();
            tokio::spawn(async move {
                match handle.await {
                    Ok(lines) => {
                        let json = serde_json::to_string(&lines).unwrap_or_else(|_| "[]".to_string());
                        if let Err(e) = task_jobs.finish(&task_job_id, json).await {
                            tracing::error!(job_id = %task_job_id, error = %e, "failed to record diff result");
                        }
                    }
                    Err(join_err) => {
                        tracing::error!(job_id = %task_job_id, error = %join_err, "diff computation panicked");
                        let _ = task_jobs
                            .fail(&task_job_id, "diff computation failed".to_string())
                            .await;
                    }
                }
            });
            Ok((StatusCode::ACCEPTED, Json(DiffJobAcceptedResponse { diff_job_id: job_id })).into_response())
        }
    }
}

/// `GET /v1/documents/{id}/diff/{diff_job_id}` — poll the async fallback.
///
/// Always 200 while the job exists, mirroring `GET /v1/jobs/{id}/result`: `lines` is
/// present only once `succeeded`, `error` only once `failed`. The `document_id` path
/// segment is not cross-checked against the job (`JobStore` has no notion of document
/// identity) — the id namespace is global, same as `/v1/jobs/{id}`.
#[utoipa::path(
    get,
    path = "/v1/documents/{id}/diff/{diff_job_id}",
    tag = "versions",
    operation_id = "getDiffJob",
    security(("bearerAuth" = [])),
    params(
        ("id" = Uuid, Path, description = "Client-supplied document id"),
        ("diff_job_id" = String, Path, description = "Job id returned by a 202 from diffDocument")
    ),
    responses(
        (status = 200, description = "Diff job status (lines present once succeeded)", body = DiffJobResultResponse),
        (status = 404, description = "No such diff job")
    )
)]
pub async fn get_diff_job(
    State(state): State<ApiState>,
    Path((_document_id, diff_job_id)): Path<(Uuid, String)>,
) -> Result<Json<DiffJobResultResponse>, ApiError> {
    let job = state
        .jobs
        .get(&diff_job_id)
        .await
        .map_err(|e| {
            tracing::error!(job_id = %diff_job_id, error = %e, "job store error fetching diff job");
            ApiError::internal()
        })?
        .ok_or_else(ApiError::not_found)?;

    let lines: Option<Vec<DiffLineDto>> = job
        .result_json
        .as_deref()
        .and_then(|s| serde_json::from_str(s).ok());

    Ok(Json(DiffJobResultResponse {
        status: job.status,
        lines,
        error: job.error,
    }))
}
