//! Public informational endpoints: health, version, info.

use axum::{extract::State, Json};

use crate::{
    dto::{HealthResponse, InfoResponse, VersionResponse},
    state::ApiState,
};

/// `GET /health` — liveness probe. Always `200 {"status": "ok"}` once the process is
/// serving; carries no dependency checks (no store reachability, no auth state).
#[utoipa::path(
    get,
    path = "/health",
    tag = "info",
    operation_id = "getHealth",
    responses((status = 200, description = "Service is healthy", body = HealthResponse))
)]
pub async fn health(_: State<ApiState>) -> Json<HealthResponse> {
    Json(HealthResponse { status: "ok" })
}

/// `GET /version` — the running binary's `CARGO_PKG_VERSION`, for clients that want to
/// confirm which server build they're talking to without parsing `/info`.
#[utoipa::path(
    get,
    path = "/version",
    tag = "info",
    operation_id = "getVersion",
    responses((status = 200, description = "Server version", body = VersionResponse))
)]
pub async fn version(_: State<ApiState>) -> Json<VersionResponse> {
    Json(VersionResponse {
        version: env!("CARGO_PKG_VERSION"),
    })
}

/// `GET /info` — static service metadata (name, version, description). No auth state,
/// no capability information — see `GET /v1/auth/whoami` for that.
#[utoipa::path(
    get,
    path = "/info",
    tag = "info",
    operation_id = "getInfo",
    responses((status = 200, description = "Service metadata", body = InfoResponse))
)]
pub async fn info(_: State<ApiState>) -> Json<InfoResponse> {
    Json(InfoResponse {
        name: "hacienda-api",
        version: env!("CARGO_PKG_VERSION"),
        description: "PII redaction and compliance API for hacienda-engine",
    })
}
