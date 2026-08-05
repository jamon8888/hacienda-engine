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
