//! Handlers for `/v1/rag/*`.
//!
//! Create/get/delete a collection, upsert a document, retrieve, list collections,
//! list a collection's documents, and kick off (plus poll) an async
//! `migrate-embeddings` job — every method `RagStore`
//! (crates/hacienda-rag/src/store.rs) exposes now has a route. `reindex_document`
//! is the one route with no dedicated `RagStore` method: it composes
//! `get_document_chunks` and `upsert_document`, both of which already have routes
//! of their own. All routes require `documents:process` and are 400
//! (`ApiError::invalid_request`) when no store is configured (`ApiState::rag_store`
//! is `None`).

use axum::{
    extract::{Path, State},
    http::{request::Parts, StatusCode},
    Json,
};
use hacienda_core::jobs::JobStatus;
use hacienda_core::tenancy::TenantCtx;
use hacienda_rag::{CollectionSpec, DocumentId, RetrieveOutput, RetrieveQuery};
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

/// Prefix a caller-facing collection name with `ctx`'s tenant before it reaches
/// `RagStore` (S1).
///
/// `hacienda-rag` is deliberately tenant-policy-free (see its crate-level doc
/// comment) — it has no `tenant_id` column or field anywhere in its trait or types.
/// Cloisonnement therefore happens here, at the API layer: every name-addressed
/// `RagStore` call goes through this so two tenants can each create a collection
/// named `"contracts"` without colliding or reading each other's data. Every handler
/// in this module that accepts or returns a collection name must scope it on the way
/// in and strip it on the way out — never let a scoped name reach a response body.
///
/// The default tenant's names are left unprefixed — same backward-compatibility rule
/// `hacienda_core::redaction::pseudonym::EnvKeyResolver` and
/// `hacienda_core::audit::segment::compute_seal_hash` both apply elsewhere in S1.
/// `POST /v1/rag/collections` shipped (unscoped) before this PR, on `main` today — a
/// deployment upgrading with existing collections must keep reaching them under their
/// original, unprefixed names, not have this migration make them invisible.
pub(crate) fn scope_collection_name(ctx: &TenantCtx, name: &str) -> String {
    if ctx.tenant.as_str() == hacienda_core::tenancy::DEFAULT_TENANT {
        name.to_owned()
    } else {
        format!("{}:{name}", ctx.tenant)
    }
}

/// Inverse of [`scope_collection_name`]: strip `ctx`'s tenant prefix from a name
/// read back from the store, returning `None` if `scoped_name` does not belong to
/// this tenant (used to filter `RagStore::list_collections`, which has no
/// tenant-aware filter of its own — see [`scope_collection_name`]'s doc comment).
///
/// For the default tenant, a name is its own — unless it contains `:`, treated as
/// belonging to a non-default tenant's scoped name instead (the only shape
/// [`scope_collection_name`] ever produces for one). A default-tenant collection
/// literally named with a `:` in it is the one edge case this cannot distinguish from
/// another tenant's scoped name — accepted for the same reason
/// `redaction::pseudonym::sanitize_env_suffix` accepts a narrower case-only collision:
/// a full reserved-character validation on collection names is a larger, separate
/// change.
fn strip_tenant_prefix(ctx: &TenantCtx, scoped_name: &str) -> Option<String> {
    if ctx.tenant.as_str() == hacienda_core::tenancy::DEFAULT_TENANT {
        if scoped_name.contains(':') {
            None
        } else {
            Some(scoped_name.to_owned())
        }
    } else {
        scoped_name
            .strip_prefix(&format!("{}:", ctx.tenant))
            .map(str::to_owned)
    }
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
    parts: Parts,
    SafeJson(spec): SafeJson<CollectionSpec>,
) -> Result<(StatusCode, Json<CollectionSpec>), ApiError> {
    let store = require_store(&state)?;
    let ctx = extract_auth_context(&parts);
    let caller = caller_from_arc(&ctx);
    let tenant_ctx = caller.tenant_ctx();

    let mut scoped_spec = spec.clone();
    scoped_spec.name = scope_collection_name(&tenant_ctx, &spec.name);
    store
        .ensure_collection(&scoped_spec)
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
    parts: Parts,
    Path(name): Path<String>,
) -> Result<Json<CollectionSpec>, ApiError> {
    let store = require_store(&state)?;
    let ctx = extract_auth_context(&parts);
    let caller = caller_from_arc(&ctx);
    let tenant_ctx = caller.tenant_ctx();

    let mut spec = store
        .get_collection(&scope_collection_name(&tenant_ctx, &name))
        .await
        .map_err(ApiError::from)?
        .ok_or_else(ApiError::not_found)?;
    spec.name = name;
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
    parts: Parts,
    Path(name): Path<String>,
) -> Result<StatusCode, ApiError> {
    let store = require_store(&state)?;
    let ctx = extract_auth_context(&parts);
    let caller = caller_from_arc(&ctx);
    let tenant_ctx = caller.tenant_ctx();

    store
        .drop_collection(&scope_collection_name(&tenant_ctx, &name))
        .await
        .map_err(ApiError::from)?;
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
    parts: Parts,
    Path(name): Path<String>,
    SafeJson(body): SafeJson<UpsertDocumentRequest>,
) -> Result<(StatusCode, Json<UpsertDocumentResponse>), ApiError> {
    let store = require_store(&state)?;
    let ctx = extract_auth_context(&parts);
    let caller = caller_from_arc(&ctx);
    let tenant_ctx = caller.tenant_ctx();

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
        .upsert_document(
            &scope_collection_name(&tenant_ctx, &name),
            &body.document,
            &chunks,
        )
        .await
        .map_err(ApiError::from)?;
    Ok((
        StatusCode::CREATED,
        Json(UpsertDocumentResponse { document_id }),
    ))
}

/// `POST /v1/rag/collections/{name}/documents/{id}/reindex` — re-derive an
/// existing document's chunks from its already-stored `full_text`, without the
/// caller resubmitting content.
///
/// Hacienda-only addition closing the xberg-sdks `reindex_rag_document` gap
/// (platform-parity design spec §3.1/§4.1). No new `RagStore` method needed:
/// this reuses `get_document_chunks` (fetch) and the same `chunk_full_text`
/// path `upsert_document` already runs when `chunks` is omitted, then
/// re-upserts under the document's own identity.
///
/// Requires the document to have been upserted with `external_id` set.
/// `RagStore::upsert_document`'s identity rule is "match by `external_id` when
/// present, otherwise insert as a new document" — silently re-upserting a
/// record that has no `external_id` would not update it in place, it would
/// create a duplicate. Returns 400 for that case rather than doing that
/// silently.
#[utoipa::path(
    post,
    path = "/v1/rag/collections/{name}/documents/{id}/reindex",
    tag = "rag",
    operation_id = "reindexDocument",
    security(("bearerAuth" = [])),
    params(
        ("name" = String, Path, description = "Collection name"),
        ("id" = String, Path, description = "Document id")
    ),
    responses(
        (status = 200, description = "Document re-chunked and re-upserted", body = UpsertDocumentResponse),
        (status = 400, description = "RAG is not enabled on this server, or the document has no external_id to reindex against"),
        (status = 404, description = "No such collection or document")
    )
)]
pub async fn reindex_document(
    State(state): State<ApiState>,
    parts: Parts,
    Path((name, id)): Path<(String, String)>,
) -> Result<Json<UpsertDocumentResponse>, ApiError> {
    let store = require_store(&state)?;
    let ctx = extract_auth_context(&parts);
    let caller = caller_from_arc(&ctx);
    let tenant_ctx = caller.tenant_ctx();
    let scoped_name = scope_collection_name(&tenant_ctx, &name);
    let document_id = DocumentId(id);

    let (document, _existing_chunks) = store
        .get_document_chunks(&scoped_name, &document_id)
        .await
        .map_err(ApiError::from)?
        .ok_or_else(ApiError::not_found)?;

    if document.external_id.is_none() {
        return Err(ApiError::invalid_request(
            "this document has no external_id, so it cannot be reindexed in place: \
             RagStore::upsert_document matches existing documents by external_id, and \
             re-upserting one without it would create a duplicate rather than update it.",
        ));
    }

    let full_text = document.full_text.clone();
    let chunks = tokio::task::spawn_blocking(move || {
        let config = hacienda_rag::ChunkingConfig::default();
        hacienda_rag::chunk_full_text(&full_text, &config)
    })
    .await
    .map_err(|_| ApiError::internal())?
    .map_err(ApiError::from)?;

    let document_id = store
        .upsert_document(&scoped_name, &document, &chunks)
        .await
        .map_err(ApiError::from)?;

    Ok(Json(UpsertDocumentResponse { document_id }))
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
    parts: Parts,
    Path(name): Path<String>,
    SafeJson(query): SafeJson<RetrieveQuery>,
) -> Result<Json<RetrieveOutput>, ApiError> {
    let store = require_store(&state)?;
    let ctx = extract_auth_context(&parts);
    let caller = caller_from_arc(&ctx);
    let tenant_ctx = caller.tenant_ctx();
    let scoped_name = scope_collection_name(&tenant_ctx, &name);

    let spec = store
        .get_collection(&scoped_name)
        .await
        .map_err(ApiError::from)?
        .ok_or_else(ApiError::not_found)?;
    query.validate(&spec).map_err(ApiError::from)?;
    let output = store
        .retrieve(&scoped_name, &query)
        .await
        .map_err(ApiError::from)?;
    Ok(Json(output))
}

/// `GET /v1/rag/collections` — list collections, paginated, newest-first.
///
/// `limit` defaults to 50 and is capped at 100, matching the real xberg-sdks
/// `ListCollectionsResponse` contract.
///
/// # Tenant scoping (S1)
///
/// `RagStore::list_collections` has no tenant-aware filter — `hacienda-rag` is
/// deliberately tenant-policy-free (see [`scope_collection_name`]) — so this pages
/// through the *entire* store, filters down to collections whose name carries the
/// caller's tenant prefix, and paginates the filtered result in-process rather than
/// pushing `limit`/`offset` straight to the backend. That is a real cost (a full
/// store scan per call) accepted for correctness: pushing pagination to the backend
/// would either leak other tenants' collections into a page or silently skip/miscount
/// this tenant's own collections whenever other tenants have any. Fine at the
/// collection counts one deployment realistically has; a store-level tenant filter
/// would need a `hacienda-rag` trait change, out of scope here.
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
    parts: Parts,
    Query(query): Query<RagListQuery>,
) -> Result<Json<ListCollectionsResponse>, ApiError> {
    let store = require_store(&state)?;
    let ctx = extract_auth_context(&parts);
    let caller = caller_from_arc(&ctx);
    let tenant_ctx = caller.tenant_ctx();

    let limit = query
        .limit
        .unwrap_or(DEFAULT_RAG_LIST_LIMIT)
        .min(MAX_RAG_LIST_LIMIT);
    let offset = query.offset.unwrap_or(0);

    const STORE_SCAN_PAGE_SIZE: u32 = 200;
    let mut matched = 0u32;
    let mut page = Vec::new();
    let mut store_offset = 0u32;
    loop {
        let (batch, store_total) = store
            .list_collections(STORE_SCAN_PAGE_SIZE, store_offset)
            .await
            .map_err(ApiError::from)?;
        if batch.is_empty() {
            break;
        }
        for mut spec in batch {
            if let Some(unscoped_name) = strip_tenant_prefix(&tenant_ctx, &spec.name) {
                if matched >= offset && (page.len() as u32) < limit {
                    spec.name = unscoped_name;
                    page.push(spec);
                }
                matched += 1;
            }
        }
        store_offset += STORE_SCAN_PAGE_SIZE;
        if u64::from(store_offset) >= store_total {
            break;
        }
    }

    Ok(Json(ListCollectionsResponse {
        collections: page,
        total: u64::from(matched),
    }))
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
    parts: Parts,
    Path(name): Path<String>,
    Query(query): Query<RagListQuery>,
) -> Result<Json<ListDocumentsResponse>, ApiError> {
    let store = require_store(&state)?;
    let ctx = extract_auth_context(&parts);
    let caller = caller_from_arc(&ctx);
    let tenant_ctx = caller.tenant_ctx();

    let limit = query
        .limit
        .unwrap_or(DEFAULT_RAG_DOCUMENT_LIST_LIMIT)
        .min(MAX_RAG_DOCUMENT_LIST_LIMIT);
    let offset = query.offset.unwrap_or(0);
    let (documents, total) = store
        .list_documents(&scope_collection_name(&tenant_ctx, &name), limit, offset)
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
    let ctx = extract_auth_context(&parts);
    let caller = caller_from_arc(&ctx);
    let tenant_ctx = caller.tenant_ctx();
    let scoped_name = scope_collection_name(&tenant_ctx, &name);

    let spec = store
        .get_collection(&scoped_name)
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

    let tenant = tenant_ctx.tenant.clone();
    let owner = caller.principal_id().map(str::to_owned);

    let job = state.jobs.create(&tenant, owner).await.map_err(|e| {
        tracing::error!(error = %e, "failed to create migrate-embeddings job");
        ApiError::internal()
    })?;
    let job_id = job.id.clone();

    let task_store = Arc::clone(store);
    let task_jobs = state.jobs.clone();
    let task_job_id = job_id.clone();
    let task_tenant = tenant.clone();
    let task_collection = scoped_name.clone();
    let task_caller_facing_name = name.clone();
    let task_to_source = body.to_source.clone();
    let task_to_version = body.to_version;

    tokio::spawn(async move {
        run_migrate_embeddings_job(
            task_store,
            task_jobs,
            task_tenant,
            task_job_id,
            task_collection,
            task_caller_facing_name,
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
///
/// Eight arguments, one per independently `.clone()`d value the caller `move`s into
/// the spawned task (see the call site) — wrapping them in a struct would just push the
/// same eight fields into a one-off type built solely to unpack them again here,
/// matching this repo's existing `#[allow(clippy::too_many_arguments)]` precedent in
/// `hacienda-core/src/audit/segment.rs`.
#[allow(clippy::too_many_arguments)]
async fn run_migrate_embeddings_job(
    store: Arc<dyn hacienda_rag::RagStore>,
    jobs: Arc<dyn hacienda_core::jobs::JobStore>,
    tenant: hacienda_core::tenancy::TenantId,
    job_id: String,
    collection: String,
    caller_facing_name: String,
    to_source: String,
    to_version: u32,
) {
    if let Err(e) = jobs
        .transition(&tenant, &job_id, JobStatus::Queued, JobStatus::Running)
        .await
    {
        tracing::error!(
            job_id = %job_id, error = %e,
            "failed to transition migrate-embeddings job to Running"
        );
        return;
    }

    match migrate_embeddings_work(&store, &jobs, &tenant, &job_id, &collection, &to_source).await {
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
                        &tenant,
                        &job_id,
                        "failed to persist embedding provenance".to_string(),
                    )
                    .await;
                return;
            }
            let result_json = serde_json::json!({
                "collection": caller_facing_name,
                "to_source": to_source,
                "to_version": to_version,
            })
            .to_string();
            if let Err(e) = jobs.finish(&tenant, &job_id, result_json).await {
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
            let _ = jobs.fail(&tenant, &job_id, message).await;
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
    tenant: &hacienda_core::tenancy::TenantId,
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
            if let Err(e) = jobs.update_progress(tenant, job_id, progress_json).await {
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
    let tenant = caller.tenant_ctx().tenant;

    let job = state
        .jobs
        .get(&tenant, &job_id)
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

#[cfg(test)]
mod tests {
    use super::*;
    use hacienda_core::auth::{authn::DevTokenResolver, AuthContext, AuthExtension, AuthState};
    use hacienda_core::jobs::InMemoryJobStore;
    use hacienda_core::tenancy::{ActorId, TenantId};
    use hacienda_core::{CapabilitySet, HaciendaConfig, HaciendaFacade};
    use hacienda_rag::{InMemoryVectorStore, RagStore};

    fn tenant_ctx(tenant: &str) -> TenantCtx {
        TenantCtx::new(TenantId::new(tenant), ActorId::new("test"))
    }

    #[test]
    fn scope_collection_name_round_trips_through_strip_tenant_prefix() {
        let ctx = tenant_ctx("acme");
        let scoped = scope_collection_name(&ctx, "contracts");
        assert_eq!(scoped, "acme:contracts");
        assert_eq!(
            strip_tenant_prefix(&ctx, &scoped).as_deref(),
            Some("contracts")
        );
    }

    #[test]
    fn strip_tenant_prefix_rejects_a_different_tenants_scoped_name() {
        let acme_scoped = scope_collection_name(&tenant_ctx("acme"), "contracts");
        assert_eq!(
            strip_tenant_prefix(&tenant_ctx("globex"), &acme_scoped),
            None
        );
    }

    /// `POST /v1/rag/collections` shipped (unscoped) before this PR. A deployment
    /// upgrading with existing default-tenant collections must keep reaching them
    /// under their original, unprefixed names.
    #[test]
    fn default_tenant_collection_names_stay_unprefixed_for_backward_compatibility() {
        let ctx = tenant_ctx(hacienda_core::tenancy::DEFAULT_TENANT);
        let scoped = scope_collection_name(&ctx, "contracts");
        assert_eq!(scoped, "contracts", "must not gain a 'default:' prefix");
        assert_eq!(
            strip_tenant_prefix(&ctx, &scoped).as_deref(),
            Some("contracts")
        );
    }

    #[test]
    fn default_tenant_does_not_see_another_tenants_scoped_collection() {
        let acme_scoped = scope_collection_name(&tenant_ctx("acme"), "contracts");
        assert_eq!(
            strip_tenant_prefix(
                &tenant_ctx(hacienda_core::tenancy::DEFAULT_TENANT),
                &acme_scoped
            ),
            None
        );
    }

    fn state_with_rag() -> ApiState {
        let facade = Arc::new(HaciendaFacade::new(HaciendaConfig::default()).unwrap());
        let jobs = InMemoryJobStore::new().into_arc();
        let auth = AuthState::new(Arc::new(DevTokenResolver)).with_enabled(false);
        let store: Arc<dyn RagStore> = Arc::new(InMemoryVectorStore::new("test"));
        ApiState::new(facade, jobs, auth, crate::state::ApiLimits::default()).with_rag_store(store)
    }

    /// Build `Parts` carrying an `AuthContext` scoped to `tenant`, the way the real auth
    /// middleware would attach one via `AuthExtension` — bypasses the token resolver
    /// pipeline entirely so the test can exercise two different tenants without needing
    /// `authn::Token`/`ApiKeyTokenResolver` to thread tenant identity (a separate,
    /// pre-existing gap: today every resolved token lands on the default tenant via
    /// `AuthContext::new` — out of scope for this RAG-collection-scoping task).
    fn parts_for(tenant: &str) -> Parts {
        let ctx = AuthContext::with_tenant(
            "test-principal",
            TenantId::new(tenant),
            CapabilitySet::new([hacienda_core::auth::Capability::DocumentsProcess]),
        );
        let mut parts = axum::http::Request::new(()).into_parts().0;
        parts.extensions.insert(AuthExtension(Arc::new(ctx)));
        parts
    }

    /// S1: two tenants each creating a collection named `"contracts"` must not collide,
    /// and neither can read, list, or delete the other's collection by name — the core
    /// property `scope_collection_name`/`strip_tenant_prefix` exist to guarantee, since
    /// `RagStore` itself has no notion of tenant (see the module doc on
    /// [`scope_collection_name`]).
    #[tokio::test]
    async fn two_tenants_can_use_the_same_collection_name_without_colliding() {
        let state = state_with_rag();
        let spec = CollectionSpec::new("contracts", 3);

        let (status, acme_spec) = create_collection(
            State(state.clone()),
            parts_for("acme"),
            SafeJson(spec.clone()),
        )
        .await
        .expect("acme create must succeed");
        assert_eq!(status, StatusCode::CREATED);
        assert_eq!(acme_spec.0.name, "contracts");

        let (status, globex_spec) =
            create_collection(State(state.clone()), parts_for("globex"), SafeJson(spec))
                .await
                .expect("globex create must succeed");
        assert_eq!(status, StatusCode::CREATED);
        assert_eq!(globex_spec.0.name, "contracts");

        // acme cannot delete globex's "contracts" by name, and vice versa: each tenant's
        // collection survives the other tenant's delete call.
        delete_collection(
            State(state.clone()),
            parts_for("acme"),
            Path("contracts".to_string()),
        )
        .await
        .expect("delete must not error");

        let globex_after_acme_delete = get_collection(
            State(state.clone()),
            parts_for("globex"),
            Path("contracts".to_string()),
        )
        .await
        .expect("globex's collection must still exist after acme deleted its own");
        assert_eq!(globex_after_acme_delete.0.name, "contracts");

        let acme_after_delete = get_collection(
            State(state.clone()),
            parts_for("acme"),
            Path("contracts".to_string()),
        )
        .await;
        assert!(
            acme_after_delete.is_err(),
            "acme's own collection must be gone after its own delete"
        );
    }

    /// `GET /v1/rag/collections` must only ever return the calling tenant's own
    /// collections, with names reported unscoped — the store-wide prefix must never
    /// leak into a response.
    #[tokio::test]
    async fn list_collections_only_returns_the_callers_own_tenant() {
        let state = state_with_rag();

        let _ = create_collection(
            State(state.clone()),
            parts_for("acme"),
            SafeJson(CollectionSpec::new("alpha", 3)),
        )
        .await
        .expect("create must succeed");
        let _ = create_collection(
            State(state.clone()),
            parts_for("acme"),
            SafeJson(CollectionSpec::new("beta", 3)),
        )
        .await
        .expect("create must succeed");
        let _ = create_collection(
            State(state.clone()),
            parts_for("globex"),
            SafeJson(CollectionSpec::new("gamma", 3)),
        )
        .await
        .expect("create must succeed");

        let acme_list = list_collections(
            State(state.clone()),
            parts_for("acme"),
            Query(RagListQuery {
                limit: None,
                offset: None,
            }),
        )
        .await
        .expect("list must succeed");
        assert_eq!(acme_list.total, 2);
        let mut acme_names: Vec<&str> = acme_list
            .collections
            .iter()
            .map(|c| c.name.as_str())
            .collect();
        acme_names.sort_unstable();
        assert_eq!(acme_names, vec!["alpha", "beta"]);

        let globex_list = list_collections(
            State(state.clone()),
            parts_for("globex"),
            Query(RagListQuery {
                limit: None,
                offset: None,
            }),
        )
        .await
        .expect("list must succeed");
        assert_eq!(globex_list.total, 1);
        assert_eq!(globex_list.collections[0].name, "gamma");
    }
}
