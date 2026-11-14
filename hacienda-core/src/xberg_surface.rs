//! Safe, shared access to the Xberg capabilities exposed by Hacienda surfaces.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// Maximum decoded input accepted by MIME detection (8 MiB).
pub const MAX_MIME_INPUT_BYTES: usize = 8 * 1024 * 1024;

use serde::Serialize;

#[derive(Debug, thiserror::Error)]
pub enum XbergSurfaceError {
    #[error("MIME input exceeds the 8 MiB limit")]
    MimeInputTooLarge,
    #[error(
        "MIME detection failed: the input bytes are not a supported document; suggestion: provide a supported format or an explicit filename hint"
    )]
    Mime(#[source] xberg::XbergError),
    #[error(
        "cache operation failed: the Xberg cache could not be accessed; suggestion: check cache permissions and XBERG_CACHE_DIR"
    )]
    Cache(#[source] xberg::XbergError),
    #[error(
        "cache operation failed: the configured Xberg cache root is unsafe; suggestion: set XBERG_CACHE_DIR to a dedicated cache directory"
    )]
    UnsafeCacheRoot,
    #[error(
        "cache operation is busy: another Xberg cache operation is still running; suggestion: retry after it completes"
    )]
    CacheBusy,
    #[error(
        "embedding warm failed: the requested preset is unavailable; suggestion: choose a compiled Xberg embedding preset"
    )]
    WarmUnavailable,
    #[error(
        "embedding warm failed: the Xberg embedding model could not be loaded; suggestion: verify ONNX Runtime and model-cache access"
    )]
    Warm(#[source] xberg::XbergError),
}

#[derive(Debug, Clone, Serialize)]
pub struct CacheStats {
    pub total_files: usize,
    pub total_size_bytes: u64,
    pub oldest_file_age_days: f64,
    pub newest_file_age_days: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct CacheClearResult {
    pub removed_files: usize,
    pub freed_bytes: u64,
    pub cleared_engine_count: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct WarmResult {
    pub preset: String,
    pub already_loaded_or_downloaded: bool,
}

pub fn supported_formats() -> Vec<xberg::SupportedFormat> {
    xberg::list_supported_formats()
}

pub fn detect_mime_type(bytes: &[u8], filename: Option<&str>) -> Result<String, XbergSurfaceError> {
    if bytes.len() > MAX_MIME_INPUT_BYTES {
        return Err(XbergSurfaceError::MimeInputTooLarge);
    }
    match xberg::detect_mime_type_from_bytes(bytes) {
        Ok(mime) => Ok(mime),
        Err(error) => filename
            .and_then(filename_mime_hint)
            .ok_or(XbergSurfaceError::Mime(error)),
    }
}

fn filename_mime_hint(filename: &str) -> Option<String> {
    let extension = Path::new(filename)
        .extension()?
        .to_str()?
        .to_ascii_lowercase();
    xberg::list_supported_formats()
        .into_iter()
        .find(|format| format.extension.eq_ignore_ascii_case(&extension))
        .map(|format| format.mime_type)
}

pub fn cache_stats() -> Result<CacheStats, XbergSurfaceError> {
    let root = cache_root();
    let root_string = root.to_string_lossy();
    let stats = xberg::cache::get_cache_metadata(&root_string).map_err(XbergSurfaceError::Cache)?;
    Ok(CacheStats {
        total_files: stats.total_files,
        total_size_bytes: (stats.total_size_mb * 1024.0 * 1024.0).round() as u64,
        oldest_file_age_days: stats.oldest_file_age_days,
        newest_file_age_days: stats.newest_file_age_days,
    })
}

pub fn clear_cache() -> Result<CacheClearResult, XbergSurfaceError> {
    let _operation = cache_operation_lock()
        .try_lock()
        .map_err(|_| XbergSurfaceError::CacheBusy)?;
    let root = cache_root();
    if !is_safe_cache_root(&root) {
        return Err(XbergSurfaceError::UnsafeCacheRoot);
    }
    let before = cache_stats()?;
    let root_string = root.to_string_lossy();
    let (removed_files, _) =
        xberg::cache::clear_cache_directory(&root_string).map_err(XbergSurfaceError::Cache)?;
    let cleared_engine_count = xberg::clear_engine_caches();
    Ok(CacheClearResult {
        removed_files,
        freed_bytes: before.total_size_bytes,
        cleared_engine_count,
    })
}

#[cfg(feature = "rag-embeddings")]
pub async fn warm_embedding_preset(_name: &str) -> Result<WarmResult, XbergSurfaceError> {
    Err(XbergSurfaceError::WarmUnavailable)
}

#[cfg(not(feature = "rag-embeddings"))]
pub async fn warm_embedding_preset(_name: &str) -> Result<WarmResult, XbergSurfaceError> {
    Err(XbergSurfaceError::WarmUnavailable)
}

fn cache_root() -> PathBuf {
    if let Some(path) = std::env::var_os("XBERG_CACHE_DIR") {
        return PathBuf::from(path);
    }
    if let Some(path) = std::env::var_os("XDG_CACHE_HOME") {
        return PathBuf::from(path).join("xberg");
    }
    if cfg!(target_os = "macos") {
        if let Some(home) = std::env::var_os("HOME") {
            return PathBuf::from(home).join("Library/Caches/xberg");
        }
    }
    if let Some(home) = std::env::var_os("HOME") {
        return PathBuf::from(home).join(".cache/xberg");
    }
    PathBuf::from(".xberg")
}

fn cache_operation_lock() -> &'static tokio::sync::Mutex<()> {
    static LOCK: OnceLock<tokio::sync::Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| tokio::sync::Mutex::new(()))
}

fn is_safe_cache_root(root: &PathBuf) -> bool {
    let Ok(canonical) = std::fs::canonicalize(root) else { return false };
    canonical.file_name().is_some_and(|name| name == "xberg")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_clear_rejects_broad_roots() {
        assert!(!is_safe_cache_root(&PathBuf::from("/")));
    }

    #[test]
    fn cache_stats_shape_does_not_expose_a_path() {
        let value = serde_json::to_value(CacheStats {
            total_files: 2,
            total_size_bytes: 128,
            oldest_file_age_days: 1.0,
            newest_file_age_days: 0.1,
        })
        .expect("cache stats serialize");
        assert!(value.get("path").is_none());
        assert!(value.get("directory").is_none());
    }
}
