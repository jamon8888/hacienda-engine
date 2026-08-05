//! Handlers for `/v1/rag/*`.
//!
//! Only the routes `RagStore` (crates/hacienda-rag/src/store.rs) can actually serve
//! are implemented here: create/get/delete a collection, upsert a document, and
//! retrieve. Listing collections/documents, per-document reindex, and
//! migrate-embeddings have no trait primitive yet — see
//! `superpowers/plans/2026-08-01-platform-parity-and-scale-implementation.md` Phase 12
//! Task 3 Step 2 for the full finding. All five routes require `documents:process`
//! and are 400 (`ApiError::invalid_request`) when no store is configured
//! (`ApiState::rag_store` is `None`).

use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use hacienda_rag::{CollectionSpec, RetrieveOutput, RetrieveQuery};

use crate::{
    dto::{UpsertDocumentRequest, UpsertDocumentResponse},
    error::ApiError,
    extract::Json as SafeJson,
    state::ApiState,
};

/// Returns the configured store, or a client-safe error if RAG is disabled.
fn require_store(
    state: &ApiState,
) -> Result<&std::sync::Arc<dyn hacienda_rag::RagStore>, ApiError> {
    state
        .rag_store
        .as_ref()
        .ok_or_else(|| ApiError::invalid_request("RAG is not enabled on this server."))
}

/// `POST /v1/rag/collections` — create a collection if it does not already exist.
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
pub async fn delete_collection(
    State(state): State<ApiState>,
    Path(name): Path<String>,
) -> Result<StatusCode, ApiError> {
    let store = require_store(&state)?;
    store.drop_collection(&name).await.map_err(ApiError::from)?;
    Ok(StatusCode::NO_CONTENT)
}

/// `POST /v1/rag/collections/{name}/documents` — upsert one document with its chunks.
pub async fn upsert_document(
    State(state): State<ApiState>,
    Path(name): Path<String>,
    SafeJson(body): SafeJson<UpsertDocumentRequest>,
) -> Result<(StatusCode, Json<UpsertDocumentResponse>), ApiError> {
    let store = require_store(&state)?;
    let document_id = store
        .upsert_document(&name, &body.document, &body.chunks)
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
