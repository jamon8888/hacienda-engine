//! Safe, shared access to the Xberg capabilities exposed by Hacienda surfaces.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// Maximum decoded input accepted by MIME detection (8 MiB).
pub const MAX_MIME_INPUT_BYTES: usize = 8 * 1024 * 1024;
/// Largest padded base64 representation of an accepted MIME input.
pub const MAX_MIME_INPUT_BASE64_BYTES: usize = MAX_MIME_INPUT_BYTES.div_ceil(3) * 4;

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
        "cache operation failed: the configured Xberg cache root is unsafe; suggestion: unset XBERG_CACHE_DIR and use the standard Xberg cache directory"
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

fn cache_directories(root: &Path) -> Result<Vec<PathBuf>, XbergSurfaceError> {
    if !root.exists() {
        return Ok(vec![root.to_path_buf()]);
    }
    let mut directories = vec![root.to_path_buf()];
    let mut pending = vec![root.to_path_buf()];
    while let Some(directory) = pending.pop() {
        let entries = std::fs::read_dir(&directory).map_err(|error| {
            XbergSurfaceError::Cache(xberg::XbergError::cache(format!(
                "failed to enumerate Xberg cache directory: {error}"
            )))
        })?;
        for entry in entries {
            let entry = entry.map_err(|error| {
                XbergSurfaceError::Cache(xberg::XbergError::cache(format!(
                    "failed to enumerate Xberg cache directory: {error}"
                )))
            })?;
            if entry.file_type().map(|kind| kind.is_dir()).unwrap_or(false) {
                let child = entry.path();
                directories.push(child.clone());
                pending.push(child);
            }
        }
    }
    Ok(directories)
}

pub fn cache_stats() -> Result<CacheStats, XbergSurfaceError> {
    let root = cache_root();
    let mut total_files = 0;
    let mut total_size_bytes = 0_u64;
    let mut oldest_file_age_days: f64 = 0.0;
    let mut newest_file_age_days = f64::INFINITY;
    for directory in cache_directories(&root)? {
        let directory_string = directory.to_string_lossy();
        let stats = xberg::cache::get_cache_metadata(&directory_string)
            .map_err(XbergSurfaceError::Cache)?;
        total_files += stats.total_files;
        total_size_bytes += (stats.total_size_mb * 1024.0 * 1024.0).round() as u64;
        oldest_file_age_days = oldest_file_age_days.max(stats.oldest_file_age_days);
        if stats.total_files > 0 {
            newest_file_age_days = newest_file_age_days.min(stats.newest_file_age_days);
        }
    }
    if total_files == 0 {
        newest_file_age_days = 0.0;
    }
    Ok(CacheStats {
        total_files,
        total_size_bytes,
        oldest_file_age_days,
        newest_file_age_days,
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
    let mut removed_files = 0;
    let mut freed_bytes = 0_u64;
    for directory in cache_directories(&root)? {
        let directory_string = directory.to_string_lossy();
        let (count, freed_mb) = xberg::cache::clear_cache_directory(&directory_string)
            .map_err(XbergSurfaceError::Cache)?;
        removed_files += count;
        freed_bytes += (freed_mb * 1024.0 * 1024.0).round() as u64;
    }
    let cleared_engine_count = xberg::clear_engine_caches();
    Ok(CacheClearResult {
        removed_files,
        freed_bytes,
        cleared_engine_count,
    })
}

#[cfg(not(feature = "rag-embeddings"))]
pub async fn warm_embedding_preset(_name: &str) -> Result<WarmResult, XbergSurfaceError> {
    Err(XbergSurfaceError::WarmUnavailable)
}

#[cfg(feature = "rag-embeddings")]
pub async fn warm_embedding_preset(name: &str) -> Result<WarmResult, XbergSurfaceError> {
    let preset = xberg::get_embedding_preset(name).ok_or(XbergSurfaceError::WarmUnavailable)?;
    let _operation = cache_operation_lock()
        .try_lock()
        .map_err(|_| XbergSurfaceError::CacheBusy)?;
    let config = xberg::EmbeddingConfig {
        model: xberg::EmbeddingModelType::Preset {
            name: preset.name.clone(),
        },
        ..Default::default()
    };
    xberg::embed_texts_async(vec!["hacienda xberg cache warmup".to_owned()], &config)
        .await
        .map_err(XbergSurfaceError::Warm)?;
    Ok(WarmResult {
        preset: preset.name,
        already_loaded_or_downloaded: false,
    })
}

fn cache_root() -> PathBuf {
    if let Some(path) = std::env::var_os("XBERG_CACHE_DIR") {
        return PathBuf::from(path);
    }
    default_cache_root()
}

fn default_cache_root() -> PathBuf {
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
    // An explicit override is intentionally read-only for this surface. Xberg itself
    // cannot prove ownership of an arbitrary override, so clearing it would risk
    // deleting unrelated files in a user-supplied directory.
    if std::env::var_os("XBERG_CACHE_DIR").is_some() {
        return false;
    }
    if root != &default_cache_root() {
        return false;
    }
    let Some(name) = root.file_name() else {
        return false;
    };
    if name != "xberg" && name != ".xberg" {
        return false;
    }
    let parent = root
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let Ok(canonical_parent) = std::fs::canonicalize(parent) else {
        return false;
    };
    let expected = canonical_parent.join(name);
    let Ok(canonical_root) = std::fs::canonicalize(root) else {
        return false;
    };
    // The final cache component must be a real directory below its canonical
    // parent. Canonicalizing both `root` and `default_cache_root()` alone would
    // accept a symlink from `xberg` to an arbitrary directory.
    canonical_root == expected
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_clear_rejects_broad_roots() {
        assert!(!is_safe_cache_root(&PathBuf::from("/")));
    }

    #[test]
    fn cache_clear_rejects_unrelated_xberg_directory() {
        let root = tempfile::tempdir().expect("temporary cache parent");
        let unrelated = root.path().join("xberg");
        std::fs::create_dir(&unrelated).expect("unrelated cache directory");
        assert!(!is_safe_cache_root(&unrelated));
    }

    #[cfg(unix)]
    #[test]
    fn cache_clear_rejects_xberg_symlink_to_unrelated_directory() {
        let root = tempfile::tempdir().expect("temporary cache parent");
        let unrelated = root.path().join("documents");
        std::fs::create_dir(&unrelated).expect("unrelated directory");
        let cache_link = root.path().join("xberg");
        std::os::unix::fs::symlink(&unrelated, &cache_link).expect("cache symlink");

        assert!(!is_safe_cache_root(&cache_link));
    }

    #[test]
    fn cache_directories_includes_nested_namespaces() {
        let root = tempfile::tempdir().expect("temporary cache parent");
        let nested = root.path().join("models").join("preset");
        std::fs::create_dir_all(&nested).expect("nested cache namespace");
        let directories = cache_directories(root.path()).expect("enumerate cache namespaces");
        assert!(directories.contains(&root.path().to_path_buf()));
        assert!(directories.contains(&root.path().join("models")));
        assert!(directories.contains(&nested));
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

    #[test]
    fn mime_detection_uses_supported_filename_as_fallback() {
        let result = detect_mime_type(&[0, 1, 2], Some("document.pdf"));
        assert_eq!(
            result.expect("supported extension fallback"),
            "application/pdf"
        );
    }
}
