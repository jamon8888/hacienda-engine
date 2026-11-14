//! Hacienda-native access to safe Xberg format, MIME, and cache operations.

use std::time::Duration;

use axum::Json;

use crate::{
    dto::{
        CacheClearRequest, CacheClearResponse, CacheStatsResponse, CacheWarmRequest,
        CacheWarmResponse, FormatsResponse, MimeDetectRequest, MimeDetectResponse,
    },
    error::ApiError,
    extract::Json as SafeJson,
};

#[utoipa::path(
    get,
    path = "/v1/formats",
    tag = "xberg",
    operation_id = "listFormats",
    responses((status = 200, description = "Supported Xberg document formats", body = FormatsResponse))
)]
pub async fn formats() -> Result<Json<FormatsResponse>, ApiError> {
    let formats = serde_json::to_value(hacienda_core::xberg_surface::supported_formats())
        .map_err(|_| ApiError::internal())?;
    Ok(Json(FormatsResponse { formats }))
}

#[utoipa::path(
    post,
    path = "/v1/mime/detect",
    tag = "xberg",
    operation_id = "detectMimeType",
    request_body = MimeDetectRequest,
    security(("bearerAuth" = [])),
    responses(
        (status = 200, description = "Detected MIME type", body = MimeDetectResponse),
        (status = 400, description = "Invalid base64 or unsupported content"),
        (status = 403, description = "Caller lacks documents:process")
    )
)]
pub async fn detect_mime(
    SafeJson(body): SafeJson<MimeDetectRequest>,
) -> Result<Json<MimeDetectResponse>, ApiError> {
    let _ = body.filename.as_deref();
    let bytes = data_encoding::BASE64
        .decode(body.content_base64.as_bytes())
        .map_err(|_| ApiError::invalid_request("content_base64 is not valid base64."))?;
    let mime_type = hacienda_core::xberg_surface::detect_mime_type(&bytes)
        .map_err(|_| ApiError::invalid_request("The content format is unsupported."))?;
    Ok(Json(MimeDetectResponse { mime_type }))
}

#[utoipa::path(
    get,
    path = "/v1/cache/stats",
    tag = "xberg",
    operation_id = "getCacheStats",
    security(("bearerAuth" = [])),
    responses((status = 200, description = "Sanitized Xberg cache statistics", body = CacheStatsResponse))
)]
pub async fn cache_stats() -> Result<Json<CacheStatsResponse>, ApiError> {
    let stats = tokio::task::spawn_blocking(hacienda_core::xberg_surface::cache_stats)
        .await
        .map_err(|_| ApiError::internal())?
        .map_err(|_| ApiError::internal())?;
    Ok(Json(CacheStatsResponse {
        total_files: stats.total_files,
        total_size_bytes: stats.total_size_bytes,
        oldest_file_age_days: stats.oldest_file_age_days,
        newest_file_age_days: stats.newest_file_age_days,
    }))
}

#[utoipa::path(
    post,
    path = "/v1/cache/clear",
    tag = "xberg",
    operation_id = "clearCache",
    request_body = CacheClearRequest,
    security(("bearerAuth" = [])),
    responses((status = 200, description = "Xberg cache cleared", body = CacheClearResponse))
)]
pub async fn cache_clear(
    SafeJson(body): SafeJson<CacheClearRequest>,
) -> Result<Json<CacheClearResponse>, ApiError> {
    if !body.confirm {
        return Err(ApiError::invalid_request(
            "Cache clear requires confirm=true.",
        ));
    }
    let result = tokio::task::spawn_blocking(hacienda_core::xberg_surface::clear_cache)
        .await
        .map_err(|_| ApiError::internal())?
        .map_err(|_| ApiError::internal())?;
    tracing::info!(
        operation = "xberg_cache_clear",
        removed_files = result.removed_files
    );
    Ok(Json(CacheClearResponse {
        removed_files: result.removed_files,
        freed_bytes: result.freed_bytes,
        cleared_engine_count: result.cleared_engine_count,
    }))
}

#[utoipa::path(
    post,
    path = "/v1/cache/warm",
    tag = "xberg",
    operation_id = "warmCache",
    request_body = CacheWarmRequest,
    security(("bearerAuth" = [])),
    responses((status = 200, description = "Embedding preset warmed", body = CacheWarmResponse))
)]
pub async fn cache_warm(
    SafeJson(body): SafeJson<CacheWarmRequest>,
) -> Result<Json<CacheWarmResponse>, ApiError> {
    if !body.confirm {
        return Err(ApiError::invalid_request(
            "Cache warm requires confirm=true.",
        ));
    }
    let result = tokio::time::timeout(
        Duration::from_secs(30 * 60),
        hacienda_core::xberg_surface::warm_embedding_preset(&body.preset),
    )
    .await
    .map_err(|_| ApiError::internal())?
    .map_err(|_| ApiError::invalid_request("The requested embedding preset is unavailable."))?;
    tracing::info!(operation = "xberg_cache_warm", preset = %result.preset);
    Ok(Json(CacheWarmResponse {
        preset: result.preset,
        already_loaded_or_downloaded: result.already_loaded_or_downloaded,
    }))
}
