//! Handler for `GET /v1/keys`.

use axum::{extract::State, http::request::Parts, Json};

use crate::{
    dto::{KeyListResponse, KeyStatusDto},
    error::ApiError,
    handlers::{caller_from_arc, extract_auth_context},
    state::ApiState,
};

/// `GET /v1/keys` — the caller's tenant's pseudonym key ids and rotation status.
/// Requires `audit:read`. Never returns key material (D-P3-3, P3b §3).
#[utoipa::path(
    get,
    path = "/v1/keys",
    tag = "pii",
    operation_id = "listKeys",
    security(("bearerAuth" = [])),
    responses(
        (status = 200, description = "Key ids and status (active/retired), no material", body = KeyListResponse),
        (status = 400, description = "No key resolver configured, or no key resolvable for the caller's tenant"),
        (status = 401, description = "Missing or invalid credentials"),
        (status = 403, description = "Caller lacks audit:read")
    )
)]
pub async fn list_keys(
    State(state): State<ApiState>,
    parts: Parts,
) -> Result<Json<KeyListResponse>, ApiError> {
    let ctx = extract_auth_context(&parts);
    let caller = caller_from_arc(&ctx);

    let keys = state
        .facade
        .list_keys_with_auth(caller)
        .await
        .map_err(ApiError::from)?
        .into_iter()
        .map(KeyStatusDto::from)
        .collect();

    Ok(Json(KeyListResponse { keys }))
}
