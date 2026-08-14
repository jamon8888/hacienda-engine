//! Handlers for `/v1/presets/*`.
//!
//! Thin CRUD over `PresetStore` (`hacienda_core::store::postgres::presets`) — presets
//! are inert config, not part of the audit-bearing pipeline, so there is no facade
//! logic here beyond capability enforcement (handled by the route table, not this
//! module). All four routes require `documents:process` and are 400
//! (`ApiError::invalid_request`) when no store is configured (`ApiState::preset_store`
//! is `None`), mirroring `handlers/rag.rs`'s `require_store` pattern exactly. Unlike
//! `RagStore`, `PresetStore` has no in-memory implementation — Postgres is its only
//! backend — so this opt-in is the only way to disable the routes at runtime.

use axum::{
    extract::{Path, State},
    http::request::Parts,
    http::StatusCode,
    Json,
};
use uuid::Uuid;

use crate::{
    dto::{CreatePresetRequest, PresetListResponse, PresetResponse},
    error::ApiError,
    extract::Json as SafeJson,
    handlers::{caller_from_arc, extract_auth_context},
    state::ApiState,
};

/// Returns the configured store, or a client-safe error if presets are disabled.
fn require_store(
    state: &ApiState,
) -> Result<&std::sync::Arc<dyn hacienda_core::store::postgres::presets::PresetStore>, ApiError> {
    state
        .preset_store
        .as_ref()
        .ok_or_else(|| ApiError::invalid_request("Presets are not enabled on this server."))
}

/// `POST /v1/presets` — create a preset.
#[utoipa::path(
    post,
    path = "/v1/presets",
    tag = "presets",
    operation_id = "createPreset",
    security(("bearerAuth" = [])),
    request_body = CreatePresetRequest,
    responses(
        (status = 201, description = "Created preset", body = PresetResponse),
        (status = 400, description = "Presets are not enabled on this server")
    )
)]
pub async fn create_preset(
    State(state): State<ApiState>,
    parts: Parts,
    SafeJson(body): SafeJson<CreatePresetRequest>,
) -> Result<(StatusCode, Json<PresetResponse>), ApiError> {
    let store = require_store(&state)?;

    let ctx = extract_auth_context(&parts);
    let caller = caller_from_arc(&ctx);
    let tenant = caller.tenant_ctx().tenant;

    let preset = store
        .create(&tenant, &body.name, body.config)
        .await
        .map_err(ApiError::from)?;
    Ok((StatusCode::CREATED, Json(PresetResponse::from(preset))))
}

/// `GET /v1/presets` — list all presets.
#[utoipa::path(
    get,
    path = "/v1/presets",
    tag = "presets",
    operation_id = "listPresets",
    security(("bearerAuth" = [])),
    responses(
        (status = 200, description = "All saved presets", body = PresetListResponse),
        (status = 400, description = "Presets are not enabled on this server")
    )
)]
pub async fn list_presets(
    State(state): State<ApiState>,
    parts: Parts,
) -> Result<Json<PresetListResponse>, ApiError> {
    let store = require_store(&state)?;

    let ctx = extract_auth_context(&parts);
    let caller = caller_from_arc(&ctx);
    let tenant = caller.tenant_ctx().tenant;

    let presets = store.list(&tenant).await.map_err(ApiError::from)?;
    Ok(Json(PresetListResponse {
        presets: presets.into_iter().map(PresetResponse::from).collect(),
    }))
}

/// `GET /v1/presets/{id}` — fetch one preset by id.
#[utoipa::path(
    get,
    path = "/v1/presets/{id}",
    tag = "presets",
    operation_id = "getPreset",
    security(("bearerAuth" = [])),
    params(("id" = Uuid, Path, description = "Preset id")),
    responses(
        (status = 200, description = "The preset", body = PresetResponse),
        (status = 400, description = "Presets are not enabled on this server"),
        (status = 404, description = "No such preset")
    )
)]
pub async fn get_preset(
    State(state): State<ApiState>,
    parts: Parts,
    Path(id): Path<Uuid>,
) -> Result<Json<PresetResponse>, ApiError> {
    let store = require_store(&state)?;

    let ctx = extract_auth_context(&parts);
    let caller = caller_from_arc(&ctx);
    let tenant = caller.tenant_ctx().tenant;

    let preset = store
        .get(&tenant, id)
        .await
        .map_err(ApiError::from)?
        .ok_or_else(ApiError::not_found)?;
    Ok(Json(PresetResponse::from(preset)))
}

/// `DELETE /v1/presets/{id}`.
#[utoipa::path(
    delete,
    path = "/v1/presets/{id}",
    tag = "presets",
    operation_id = "deletePreset",
    security(("bearerAuth" = [])),
    params(("id" = Uuid, Path, description = "Preset id")),
    responses(
        (status = 204, description = "Deleted"),
        (status = 400, description = "Presets are not enabled on this server")
    )
)]
pub async fn delete_preset(
    State(state): State<ApiState>,
    parts: Parts,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    let store = require_store(&state)?;

    let ctx = extract_auth_context(&parts);
    let caller = caller_from_arc(&ctx);
    let tenant = caller.tenant_ctx().tenant;

    store.delete(&tenant, id).await.map_err(ApiError::from)?;
    Ok(StatusCode::NO_CONTENT)
}
