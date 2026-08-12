//! Handlers for `/v1/rag/*`.
//!
//! Create/get/delete a collection, upsert a document, retrieve, list collections,
//! list a collection's documents, and kick off (plus poll) an async
//! `migrate-embeddings` job — every method `RagStore`
//! (crates/hacienda-rag/src/store.rs) exposes now has a route. All routes require
//! `documents:process` and are 400 (`ApiError::invalid_request`) when no store is
//! configured (`ApiState::rag_store` is `None`).

use axum::{
    extract::{Path, State},
    http::{request::Parts, StatusCode},
    Json,
};
use hacienda_core::jobs::JobStatus;
use hacienda_rag::{CollectionSpec, RetrieveOutput, RetrieveQuery};
use std::sync::Arc;

use crate::{
    dto::{
        ListCollectionsResponse, ListDocumentsResponse, MigrateEmbeddingsRequest,
        MigrateEmbeddingsResponse, MigrateProgressDto, MigrateStatusResponse, RagListQuery,
        UpsertDocumentRequest, UpsertDocumentResponse,
    },
    error::ApiError,
    extract::{Json as SafeJson, Query},
    handlers::{caller_from_arc, extract_auth_context},
    state::ApiState,
};

/// Default page size for `GET /v1/rag/collections` when `limit` is omitted.
const DEFAULT_RAG_LIST_LIMIT: u32 = 50;

/// Upper bound on `limit` for `GET /v1/rag/collections`, matching the real
/// xberg-sdks `ListCollectionsResponse` contract.
const MAX_RAG_LIST_LIMIT: u32 = 100;

/// Default page size for `GET /v1/rag/collections/{name}/documents` when
/// `limit` is omitted. No upstream contract exists for this route (a
/// hacienda-only addition — see the module docs), so this mirrors
/// `GET /v1/jobs`'s own default/cap (`handlers/jobs.rs`) rather than RAG's
/// own numbers above, which are pinned to the real spec.
const DEFAULT_RAG_DOCUMENT_LIST_LIMIT: u32 = 50;

/// Upper bound on `limit` for `GET /v1/rag/collections/{name}/documents`.
const MAX_RAG_DOCUMENT_LIST_LIMIT: u32 = 200;

/// Page size used internally by [`migrate_embeddings_work`] when paging
/// through a collection's documents via `RagStore::list_documents`. Not
/// caller-configurable — an implementation detail of the background job, not
/// a wire parameter.
const MIGRATE_PAGE_SIZE: u32 = 50;

/// Returns the configured store, or a client-safe error if RAG is disabled.
///
/// `pub(crate)`, not private: `handlers::rag_stream::answer` (Phase 12 Track 3)
/// reuses this rather than duplicating the "RAG is not enabled" error.
pub(crate) fn require_store(
    state: &ApiState,
) -> Result<&std::sync::Arc<dyn hacienda_rag::RagStore>, ApiError> {
    state
        .rag_store
        .as_ref()
        .ok_or_else(|| ApiError::invalid_request("RAG is not enabled on this server."))
}

/// `POST /v1/rag/collections` — create a collection if it does not already exist.
#[utoipa::path(
    post,
    path = "/v1/rag/collections",
    tag = "rag",
    operation_id = "createCollection",
    security(("bearerAuth" = [])),
    request_body = serde_json::Value,
    responses(
        (status = 201, description = "Collection created (idempotent if it already exists)", body = serde_json::Value),
        (status = 400, description = "RAG is not enabled on this server")
    )
)]
pub async fn create_collection(
    State(state): State<ApiState>,
    SafeJson(spec): SafeJson<CollectionSpec>,
) -> Result<(StatusCode, Json<CollectionSpec>), ApiError> {
    let store = require_store(&state)?;
    store
        .ensure_collection(&spec)
        .await
        .map_err(ApiError::from)?;
    Ok((StatusCode::CREATED, Json(spec)))
}

/// `GET /v1/rag/collections/{name}`.
#[utoipa::path(
    get,
    path = "/v1/rag/collections/{name}",
    tag = "rag",
    operation_id = "getCollection",
    security(("bearerAuth" = [])),
    params(("name" = String, Path, description = "Collection name")),
    responses(
        (status = 200, description = "The collection spec", body = serde_json::Value),
        (status = 400, description = "RAG is not enabled on this server"),
        (status = 404, description = "No such collection")
    )
)]
pub async fn get_collection(
    State(state): State<ApiState>,
    Path(name): Path<String>,
) -> Result<Json<CollectionSpec>, ApiError> {
    let store = require_store(&state)?;
    let spec = store
        .get_collection(&name)
        .await
        .map_err(ApiError::from)?
        .ok_or_else(ApiError::not_found)?;
    Ok(Json(spec))
}

/// `DELETE /v1/rag/collections/{name}`.
#[utoipa::path(
    delete,
    path = "/v1/rag/collections/{name}",
    tag = "rag",
    operation_id = "deleteCollection",
    security(("bearerAuth" = [])),
    params(("name" = String, Path, description = "Collection name")),
    responses(
        (status = 204, description = "Deleted"),
        (status = 400, description = "RAG is not enabled on this server")
    )
)]
pub async fn delete_collection(
    State(state): State<ApiState>,
    Path(name): Path<String>,
) -> Result<StatusCode, ApiError> {
    let store = require_store(&state)?;
    store.drop_collection(&name).await.map_err(ApiError::from)?;
    Ok(StatusCode::NO_CONTENT)
}

/// `POST /v1/rag/collections/{name}/documents` — upsert one document with its chunks.
///
/// `chunks` may be omitted (or empty): the server then splits `document.full_text`
/// itself via `hacienda_rag::chunk_full_text`, producing chunks with `content` set and
/// `embedding` empty — a collection used for full-text/keyword retrieval alone has no
/// need of one. A caller that wants embedded chunks either submits pre-embedded
/// `chunks` itself, or re-embeds after upsert (the same `xberg::embed_texts_async` path
/// `migrate_embeddings` already uses, behind the `rag-embeddings` build feature).
#[utoipa::path(
    post,
    path = "/v1/rag/collections/{name}/documents",
    tag = "rag",
    operation_id = "upsertDocument",
    security(("bearerAuth" = [])),
    params(("name" = String, Path, description = "Collection name")),
    request_body = UpsertDocumentRequest,
    responses(
        (status = 201, description = "Document upserted", body = UpsertDocumentResponse),
        (status = 400, description = "RAG is not enabled on this server")
    )
)]
pub async fn upsert_document(
    State(state): State<ApiState>,
    Path(name): Path<String>,
    SafeJson(body): SafeJson<UpsertDocumentRequest>,
) -> Result<(StatusCode, Json<UpsertDocumentResponse>), ApiError> {
    let store = require_store(&state)?;

    let chunks = if body.chunks.is_empty() {
        // Off the async runtime: `chunk_text` is synchronous, CPU-bound work over
        // `full_text` (unbounded caller input), and running it inline on a request's
        // worker thread would block every other task scheduled on it for the duration —
        // unlike the audit handlers' deliberately-inline blocking reads (see
        // `handlers::audit`'s module doc), this sits on the document-ingestion hot path.
        let full_text = body.document.full_text.clone();
        tokio::task::spawn_blocking(move || {
            let config = hacienda_rag::ChunkingConfig::default();
            hacienda_rag::chunk_full_text(&full_text, &config)
        })
        .await
        .map_err(|_| ApiError::internal())?
        .map_err(ApiError::from)?
    } else {
        body.chunks
    };

    let document_id = store
        .upsert_document(&name, &body.document, &chunks)
        .await
        .map_err(ApiError::from)?;
    Ok((
        StatusCode::CREATED,
        Json(UpsertDocumentResponse { document_id }),
    ))
}

/// `POST /v1/rag/collections/{name}/retrieve`.
///
/// Fetches the collection spec first so `RetrieveQuery::validate` can check
/// `top_k`, filter complexity, and query-vector dimension against it before the
/// query reaches the store — the same validate-then-call shape
/// `RetrieveQuery::validate`'s own doc comment describes.
#[utoipa::path(
    post,
    path = "/v1/rag/collections/{name}/retrieve",
    tag = "rag",
    operation_id = "retrieve",
    security(("bearerAuth" = [])),
    params(("name" = String, Path, description = "Collection name")),
    request_body = serde_json::Value,
    responses(
        (status = 200, description = "Retrieved chunks", body = serde_json::Value),
        (status = 400, description = "RAG is not enabled, or the query is invalid for this collection"),
        (status = 404, description = "No such collection")
    )
)]
pub async fn retrieve(
    State(state): State<ApiState>,
    Path(name): Path<String>,
    SafeJson(query): SafeJson<RetrieveQuery>,
) -> Result<Json<RetrieveOutput>, ApiError> {
    let store = require_store(&state)?;
    let spec = store
        .get_collection(&name)
        .await
        .map_err(ApiError::from)?
        .ok_or_else(ApiError::not_found)?;
    query.validate(&spec).map_err(ApiError::from)?;
    let output = store
        .retrieve(&name, &query)
        .await
        .map_err(ApiError::from)?;
    Ok(Json(output))
}

/// `GET /v1/rag/collections` — list collections, paginated, newest-first.
///
/// `limit` defaults to 50 and is capped at 100, matching the real xberg-sdks
/// `ListCollectionsResponse` contract. Pagination is pushed into the backend
/// (`RagStore::list_collections`), not sliced in-process — see that method's
/// doc comment for why.
#[utoipa::path(
    get,
    path = "/v1/rag/collections",
    tag = "rag",
    operation_id = "listCollections",
    security(("bearerAuth" = [])),
    params(
        ("limit" = Option<u32>, Query, description = "Max collections to return (default 50, capped at 100)"),
        ("offset" = Option<u32>, Query, description = "Collections to skip before the returned page")
    ),
    responses(
        (status = 200, description = "Page of collections", body = ListCollectionsResponse),
        (status = 400, description = "RAG is not enabled on this server")
    )
)]
pub async fn list_collections(
    State(state): State<ApiState>,
    Query(query): Query<RagListQuery>,
) -> Result<Json<ListCollectionsResponse>, ApiError> {
    let store = require_store(&state)?;
    let limit = query
        .limit
        .unwrap_or(DEFAULT_RAG_LIST_LIMIT)
        .min(MAX_RAG_LIST_LIMIT);
    let offset = query.offset.unwrap_or(0);
    let (collections, total) = store
        .list_collections(limit, offset)
        .await
        .map_err(ApiError::from)?;
    Ok(Json(ListCollectionsResponse { collections, total }))
}

/// `GET /v1/rag/collections/{name}/documents` — list a collection's documents,
/// paginated, newest-first.
///
/// No upstream contract: the real xberg-sdks OpenAPI spec has no equivalent
/// route, so `limit`'s default/cap here are a hacienda-only judgment call
/// (see [`DEFAULT_RAG_DOCUMENT_LIST_LIMIT`]).
#[utoipa::path(
    get,
    path = "/v1/rag/collections/{name}/documents",
    tag = "rag",
    operation_id = "listDocuments",
    security(("bearerAuth" = [])),
    params(
        ("name" = String, Path, description = "Collection name"),
        ("limit" = Option<u32>, Query, description = "Max documents to return (default 50, capped at 200)"),
        ("offset" = Option<u32>, Query, description = "Documents to skip before the returned page")
    ),
    responses(
        (status = 200, description = "Page of a collection's documents", body = ListDocumentsResponse),
        (status = 400, description = "RAG is not enabled on this server")
    )
)]
pub async fn list_documents(
    State(state): State<ApiState>,
    Path(name): Path<String>,
    Query(query): Query<RagListQuery>,
) -> Result<Json<ListDocumentsResponse>, ApiError> {
    let store = require_store(&state)?;
    let limit = query
        .limit
        .unwrap_or(DEFAULT_RAG_DOCUMENT_LIST_LIMIT)
        .min(MAX_RAG_DOCUMENT_LIST_LIMIT);
    let offset = query.offset.unwrap_or(0);
    let (documents, total) = store
        .list_documents(&name, limit, offset)
        .await
        .map_err(ApiError::from)?;
    Ok(Json(ListDocumentsResponse { documents, total }))
}

/// `POST /v1/rag/collections/{name}/migrate-embeddings` — re-embed every
/// document in a collection under a new embedding source/version, in a
/// detached background job. Returns `202 Accepted` immediately with a
/// `job_id` to poll at `GET .../migrate-embeddings/{job_id}`.
///
/// Validation runs synchronously, before any job is created, so a bad
/// request never produces a job that is created only to immediately fail:
///
/// - the collection must exist (404),
/// - `to_source` must name a real preset per `xberg::get_embedding_preset`
///   (400 if unknown — this only reads static preset metadata, no ONNX or
///   network access happens at this point),
/// - the preset's `dimensions` must match `CollectionSpec::embedding_dim`
///   (400 — a collection's vector storage has a fixed dimension; migrating to
///   a different-dimension preset would require re-creating the collection,
///   which is out of scope for this route),
/// - `to_version` must be strictly greater than the collection's current
///   `embedding_version` (400 — versions only move forward; a caller aiming
///   at the current or a past version almost certainly has a stale read of
///   collection state, and accepting it would let two concurrent migrations
///   race to record the same version number).
///
/// The job itself only does real re-embedding if this binary was built with
/// `--features rag-embeddings` (see `hacienda-api/Cargo.toml`) — that feature
/// pulls in ONNX Runtime via `xberg`/`hacienda-rag`'s own `embeddings`
/// feature, and is off by default because `ort`'s accelerator selection is
/// not portable to every build host. Without it, validation above still runs
/// (preset lookup only needs the always-on, ORT-free `embedding-presets`
/// feature), the job is created and transitions to `Running`, and then fails
/// with a clear "requires the `embeddings` ... feature" error from
/// `xberg::embed_texts_async`'s stub — never a silent no-op.
#[utoipa::path(
    post,
    path = "/v1/rag/collections/{name}/migrate-embeddings",
    tag = "rag",
    operation_id = "migrateEmbeddings",
    security(("bearerAuth" = [])),
    params(("name" = String, Path, description = "Collection name")),
    request_body = MigrateEmbeddingsRequest,
    responses(
        (status = 202, description = "Migration job accepted; poll the returned path", body = MigrateEmbeddingsResponse),
        (status = 400, description = "RAG not enabled, unknown preset, dimension mismatch, or to_version not greater than current"),
        (status = 404, description = "No such collection")
    )
)]
pub async fn migrate_embeddings(
    State(state): State<ApiState>,
    parts: Parts,
    Path(name): Path<String>,
    SafeJson(body): SafeJson<MigrateEmbeddingsRequest>,
) -> Result<(StatusCode, Json<MigrateEmbeddingsResponse>), ApiError> {
    let store = require_store(&state)?;

    let spec = store
        .get_collection(&name)
        .await
        .map_err(ApiError::from)?
        .ok_or_else(ApiError::not_found)?;

    let preset = xberg::get_embedding_preset(&body.to_source).ok_or_else(|| {
        ApiError::invalid_request(format!("Unknown embedding preset '{}'.", body.to_source))
    })?;

    if preset.dimensions as u32 != spec.embedding_dim {
        return Err(ApiError::invalid_request(format!(
            "Preset '{}' produces {}-dimensional vectors, but collection '{}' is configured \
             for {} dimensions. Migrating to a different-dimension preset requires re-creating \
             the collection.",
            body.to_source, preset.dimensions, name, spec.embedding_dim
        )));
    }

    if body.to_version <= spec.embedding_version {
        return Err(ApiError::invalid_request(format!(
            "to_version ({}) must be greater than the collection's current embedding_version ({}).",
            body.to_version, spec.embedding_version
        )));
    }

    let ctx = extract_auth_context(&parts);
    let caller = caller_from_arc(&ctx);
    let owner = caller.principal_id().map(str::to_owned);

    let job = state
        .jobs
        .create(&caller.tenant_ctx(), owner)
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "failed to create migrate-embeddings job");
            ApiError::internal()
        })?;
    let job_id = job.id.clone();

    let task_store = Arc::clone(store);
    let task_jobs = state.jobs.clone();
    let task_job_id = job_id.clone();
    let task_collection = name.clone();
    let task_to_source = body.to_source.clone();
    let task_to_version = body.to_version;

    tokio::spawn(async move {
        run_migrate_embeddings_job(
            task_store,
            task_jobs,
            task_job_id,
            task_collection,
            task_to_source,
            task_to_version,
        )
        .await;
    });

    Ok((
        StatusCode::ACCEPTED,
        Json(MigrateEmbeddingsResponse {
            job_id: job_id.clone(),
            collection_id: name.clone(),
            from_source: spec.embedding_source,
            to_source: body.to_source,
            from_version: spec.embedding_version,
            to_version: body.to_version,
            status: JobStatus::Queued,
            poll: format!("/v1/rag/collections/{name}/migrate-embeddings/{job_id}"),
        }),
    ))
}

/// Runs a `migrate-embeddings` job to completion: transitions the job to
/// `Running`, delegates the actual re-embedding work to
/// [`migrate_embeddings_work`], and finishes or fails the job with the
/// outcome. Split out of [`migrate_embeddings`] so the handler itself stays a
/// thin validate-then-spawn shim, matching
/// `documents::process_documents_async`'s shape.
async fn run_migrate_embeddings_job(
    store: Arc<dyn hacienda_rag::RagStore>,
    jobs: Arc<dyn hacienda_core::jobs::JobStore>,
    job_id: String,
    collection: String,
    to_source: String,
    to_version: u32,
) {
    if let Err(e) = jobs
        .transition(&job_id, JobStatus::Queued, JobStatus::Running)
        .await
    {
        tracing::error!(
            job_id = %job_id, error = %e,
            "failed to transition migrate-embeddings job to Running"
        );
        return;
    }

    match migrate_embeddings_work(&store, &jobs, &job_id, &collection, &to_source).await {
        Ok(()) => {
            if let Err(e) = store
                .set_embedding_provenance(&collection, &to_source, to_version)
                .await
            {
                tracing::error!(
                    job_id = %job_id, collection = %collection, error = %e,
                    "migrate-embeddings succeeded but failed to persist provenance"
                );
                let _ = jobs
                    .fail(
                        &job_id,
                        "failed to persist embedding provenance".to_string(),
                    )
                    .await;
                return;
            }
            let result_json = serde_json::json!({
                "collection": collection,
                "to_source": to_source,
                "to_version": to_version,
            })
            .to_string();
            if let Err(e) = jobs.finish(&job_id, result_json).await {
                tracing::error!(
                    job_id = %job_id, error = %e,
                    "failed to mark migrate-embeddings job as succeeded"
                );
            }
        }
        Err(message) => {
            tracing::error!(
                job_id = %job_id, collection = %collection, error = %message,
                "migrate-embeddings job failed"
            );
            let _ = jobs.fail(&job_id, message).await;
        }
    }
}

/// Pages through `collection`'s documents, re-embeds every chunk's content
/// under `to_source`, and writes the new vectors back via
/// `RagStore::update_chunk_embeddings`. Reports progress via
/// `JobStore::update_progress` after each page.
///
/// Returns `Err(message)` — a short, client-safe (no document content)
/// description — on the first failure; `run_migrate_embeddings_job` records
/// it via `JobStore::fail`.
async fn migrate_embeddings_work(
    store: &Arc<dyn hacienda_rag::RagStore>,
    jobs: &Arc<dyn hacienda_core::jobs::JobStore>,
    job_id: &str,
    collection: &str,
    to_source: &str,
) -> Result<(), String> {
    let (_, total) = store
        .list_documents(collection, 1, 0)
        .await
        .map_err(|e| format!("failed to read collection document count: {e}"))?;

    let config = xberg::EmbeddingConfig {
        model: xberg::EmbeddingModelType::Preset {
            name: to_source.to_string(),
        },
        ..Default::default()
    };

    let mut offset = 0u32;
    let mut done = 0u64;

    loop {
        let (page, _) = store
            .list_documents(collection, MIGRATE_PAGE_SIZE, offset)
            .await
            .map_err(|e| format!("failed to list documents: {e}"))?;
        if page.is_empty() {
            break;
        }

        for summary in &page {
            let Some((_, chunks)) = store
                .get_document_chunks(collection, &summary.id)
                .await
                .map_err(|e| format!("failed to read document chunks: {e}"))?
            else {
                // Deleted between listing and fetching — not a failure, just skip.
                continue;
            };

            if !chunks.is_empty() {
                let texts: Vec<String> = chunks.iter().map(|c| c.content.clone()).collect();
                let vectors = xberg::embed_texts_async(texts, &config)
                    .await
                    .map_err(|e| format!("embedding failed: {e}"))?;

                // Defensive: `embed_texts_async` is not contractually guaranteed to
                // return one vector per input text. Catch a mismatch here rather
                // than silently zipping a short/long vector list against ordinals
                // and losing (or misattributing) embeddings for a subset of chunks.
                if vectors.len() != chunks.len() {
                    return Err(format!(
                        "embedding backend returned {} vectors for {} chunks",
                        vectors.len(),
                        chunks.len()
                    ));
                }

                let embeddings: Vec<(u32, Vec<f32>)> =
                    chunks.iter().map(|c| c.ordinal).zip(vectors).collect();

                store
                    .update_chunk_embeddings(collection, &summary.id, &embeddings)
                    .await
                    .map_err(|e| format!("failed to write updated embeddings: {e}"))?;
            }

            done += 1;
        }

        let progress = MigrateProgressDto {
            documents_dual_written: done,
            documents_total: total,
            current_phase: "embedding".to_string(),
        };
        if let Ok(progress_json) = serde_json::to_string(&progress) {
            if let Err(e) = jobs.update_progress(job_id, progress_json).await {
                tracing::warn!(
                    job_id = %job_id, error = %e,
                    "failed to record migrate-embeddings progress"
                );
            }
        }

        offset += MIGRATE_PAGE_SIZE;
    }

    Ok(())
}

/// `GET /v1/rag/collections/{name}/migrate-embeddings/{job_id}` — poll a
/// migrate-embeddings job.
///
/// Applies the same 404-not-403 ownership check as `GET /v1/jobs/{id}` (see
/// `handlers/jobs.rs`'s module docs for the membership-oracle rationale) —
/// deliberately stricter than `versions::get_diff_job`'s poll endpoint, which
/// applies no ownership check at all. A migrate-embeddings job can run for
/// minutes against a large collection; that is worth the same protection as
/// any other job, even though `get_diff_job`'s existing behavior does not
/// currently match it.
///
/// The `name` path segment is not cross-checked against the job (`JobStore`
/// has no notion of collection identity) — same as `get_diff_job`'s
/// `document_id`.
#[utoipa::path(
    get,
    path = "/v1/rag/collections/{name}/migrate-embeddings/{job_id}",
    tag = "rag",
    operation_id = "getMigrateStatus",
    security(("bearerAuth" = [])),
    params(
        ("name" = String, Path, description = "Collection name"),
        ("job_id" = String, Path, description = "Job id returned by migrateEmbeddings")
    ),
    responses(
        (status = 200, description = "Migration job status and progress", body = MigrateStatusResponse),
        (status = 404, description = "No such job, or not owned by the caller")
    )
)]
pub async fn get_migrate_status(
    State(state): State<ApiState>,
    parts: Parts,
    Path((_name, job_id)): Path<(String, String)>,
) -> Result<Json<MigrateStatusResponse>, ApiError> {
    let ctx = extract_auth_context(&parts);
    let caller = caller_from_arc(&ctx);

    let job = state
        .jobs
        .get(&job_id)
        .await
        .map_err(|e| {
            tracing::error!(
                job_id = %job_id, error = %e,
                "job store error fetching migrate-embeddings job"
            );
            ApiError::internal()
        })?
        .ok_or_else(ApiError::not_found)?;

    // Ownership check: see this function's doc and `handlers/jobs.rs`'s module docs
    // for the 404-not-403 rationale.
    match (caller.principal_id(), &job.owner) {
        (None, _) => {}
        (Some(caller_id), Some(owner_id)) if caller_id == owner_id => {}
        _ => {
            return Err(ApiError::not_found());
        }
    }

    let progress: Option<MigrateProgressDto> = job
        .progress_json
        .as_deref()
        .and_then(|s| serde_json::from_str(s).ok());

    Ok(Json(MigrateStatusResponse {
        status: job.status,
        progress,
        error: job.error,
    }))
}
