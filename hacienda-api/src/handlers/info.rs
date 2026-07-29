//! Public informational endpoints: health, version, info.

use axum::{extract::State, Json};

use crate::{
    dto::{HealthResponse, InfoResponse, VersionResponse},
    state::ApiState,
};

pub async fn health(_: State<ApiState>) -> Json<HealthResponse> {
    Json(HealthResponse { status: "ok" })
}

pub async fn version(_: State<ApiState>) -> Json<VersionResponse> {
    Json(VersionResponse {
        version: env!("CARGO_PKG_VERSION"),
    })
}

pub async fn info(_: State<ApiState>) -> Json<InfoResponse> {
    Json(InfoResponse {
        name: "hacienda-api",
        version: env!("CARGO_PKG_VERSION"),
        description: "PII redaction and compliance API for hacienda-engine",
    })
}
