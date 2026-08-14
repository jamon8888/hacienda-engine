//! Handler for `GET /v1/usage`.
//!
//! `UsageStore` has no in-memory backend (Postgres-only) — like `PresetStore`,
//! `DocumentVersionStore`, and `ObjectStore`, this route 400s when
//! `ApiState::usage_store` is `None`.

use axum::{extract::State, Json};
use std::sync::Arc;

use hacienda_core::store::postgres::usage::UsageStore;

use crate::{dto::UsageResponse, error::ApiError, extract::Query, state::ApiState};

fn require_store(state: &ApiState) -> Result<&Arc<dyn UsageStore>, ApiError> {
    state
        .usage_store
        .as_ref()
        .ok_or_else(|| ApiError::invalid_request("Usage metering is not enabled on this server."))
}

/// `GET /v1/usage?since=&until=` — per-principal entity/byte counts over the audit
/// chain, optionally windowed by `created_at`. Both bounds are optional and independent.
///
/// # Known gap: not tenant-scoped
///
/// This route currently returns usage aggregated across **every** tenant, not just the
/// caller's — a deliberately deferred, tracked gap in
/// [`UsageStore::summary`](hacienda_core::store::postgres::usage::UsageStore::summary),
/// whose own doc has the full rationale. The response carries `principal` values, so
/// this discloses which principals exist in other tenants, not only usage totals.
#[utoipa::path(
    get,
    path = "/v1/usage",
    tag = "usage",
    operation_id = "getUsage",
    security(("bearerAuth" = [])),
    params(
        ("since" = Option<String>, Query, description = "Window start (RFC 3339), inclusive"),
        ("until" = Option<String>, Query, description = "Window end (RFC 3339), exclusive")
    ),
    responses(
        (status = 200, description = "Per-principal entity/byte counts", body = UsageResponse),
        (status = 400, description = "Usage metering is not enabled on this server")
    )
)]
pub async fn get_usage(
    State(state): State<ApiState>,
    Query(query): Query<crate::dto::UsageQuery>,
) -> Result<Json<UsageResponse>, ApiError> {
    let store = require_store(&state)?;
    let records = store
        .summary(query.since, query.until)
        .await
        .map_err(ApiError::from)?;

    Ok(Json(UsageResponse {
        records: records.into_iter().map(Into::into).collect(),
        since: query.since,
        until: query.until,
    }))
}
