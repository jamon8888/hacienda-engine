//! Pinned model identity and asset verification.
//!
//! Model acquisition stays in the host (HuggingFace download, IndexedDB cache,
//! storage quota, user consent). This module owns only the **contract** a host
//! must satisfy: a model id, the SHA-256 of each of its assets, and a verifier
//! that accepts a local directory. No network access lives here, so the crate
//! is safe for native CLI, API, Tauri hosts and offline deployments.
//!
//! The registry is validated and embedded at build time from
//! `model-assets/registry.json`, the same file Studio's `model-digests.ts`
//! imports. An invalid registry fails the build — it is never silently
//! replaced by an empty one. One registry, two hosts.
//! Verification streams files: the pinned model alone is ~1.2 GB.

use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::fmt;
use std::fs::{self, File};
use std::io::{self, BufReader, Read};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use thiserror::Error;

const BUFFER_SIZE: usize = 1024 * 1024;

/// Model used when a host does not override its model id.
pub const DEFAULT_MODEL_ID: &str = "jamon8888/gliner2-guardrails-pii-f16";

/// One pinned file of a model.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct PinnedAsset {
    /// Stable role name used by callers (`model`, `tokenizer`, `encoder`, ...).
    pub role: String,
    /// Path relative to the model directory.
    pub path: String,
    /// Full SHA-256 of the file.
    pub sha256: String,
    /// Exact byte length when the host pins one.
    #[serde(default)]
    pub size: Option<u64>,
}

/// A registered model and all of its pinned assets.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct PinnedModel {
    pub id: String,
    #[serde(default)]
    pub revision: Option<String>,
    pub assets: Vec<PinnedAsset>,
}

#[derive(Debug, Clone, Deserialize)]
struct Registry {
    version: u32,
    models: Vec<PinnedModel>,
}

// Parsed and validated by `build.rs`; the generated values are typed statics,
// so initialization cannot fail and needs no panic path.
include!(concat!(env!("OUT_DIR"), "/registry_generated.rs"));

fn registry() -> &'static Registry {
    static REGISTRY: OnceLock<Registry> = OnceLock::new();
    REGISTRY.get_or_init(validated_registry)
}

/// Registry schema version, bumped when roles or filenames change.
pub fn registry_version() -> u32 {
    registry().version
}

/// Whether a model id is pinned in the shared registry.
pub fn is_known_model(model_id: &str) -> bool {
    model_entry(model_id).is_some()
}

/// Full pinned entry for `model_id`, if registered.
pub fn model_entry(model_id: &str) -> Option<&'static PinnedModel> {
    registry().models.iter().find(|entry| entry.id == model_id)
}

/// The three digests Studio needs, derived from the same pinned assets.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssetDigests {
    pub model: String,
    pub tokenizer: String,
    pub encoder: String,
}

/// Convenience view for hosts that only load the three inference assets.
pub fn digests_for(model_id: &str) -> Option<AssetDigests> {
    let entry = model_entry(model_id)?;
    let digest = |role: &str| -> Option<String> {
        entry
            .assets
            .iter()
            .find(|asset| asset.role == role)
            .map(|asset| asset.sha256.clone())
    };
    Some(AssetDigests {
        model: digest("model")?,
        tokenizer: digest("tokenizer")?,
        encoder: digest("encoder")?,
    })
}

/// The verification step that failed, retained so callers can tell metadata
/// inspection, opening and byte-streaming failures apart.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssetOperation {
    /// Inspecting file metadata before hashing (existence and size checks).
    InspectMetadata,
    /// Opening the pinned asset file.
    Open,
    /// Streaming the asset bytes through the SHA-256 hasher.
    Stream,
}

impl fmt::Display for AssetOperation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::InspectMetadata => "inspecting metadata",
            Self::Open => "opening",
            Self::Stream => "streaming",
        })
    }
}

/// Errors while checking that local bytes match the pinned model identity.
///
/// Every variant keeps the operation, the input, the root cause (where one
/// exists) and a recovery suggestion. The display text is safe to log; hosts
/// may still map it to a generic UI message.
#[derive(Debug, Error)]
pub enum ModelAssetsError {
    #[error(
        "model '{0}' is not pinned in the shared registry; re-download the pinned model or update the registry"
    )]
    UnknownModel(String),

    #[error("model asset {path} (role: {role}) is missing; re-download the pinned model")]
    MissingAsset { role: String, path: PathBuf },

    #[error(
        "model asset {path} has size {actual}, expected {expected}; re-download the pinned asset"
    )]
    SizeMismatch {
        path: PathBuf,
        expected: u64,
        actual: u64,
    },

    #[error(
        "model asset {path}: {operation} failed: {source}; check file permissions or re-download the pinned asset"
    )]
    Read {
        operation: AssetOperation,
        path: PathBuf,
        #[source]
        source: io::Error,
    },

    #[error(
        "SHA-256 mismatch for {role} of '{model_id}': expected {expected}, got {actual}; re-download the pinned asset"
    )]
    DigestMismatch {
        model_id: String,
        role: String,
        expected: String,
        actual: String,
    },
}

#[cfg(test)]
fn verify_bytes(
    model_id: &str,
    role: &str,
    bytes: &[u8],
    expected: &str,
) -> Result<(), ModelAssetsError> {
    let actual = hex_digest(bytes);
    if actual == expected {
        return Ok(());
    }
    Err(ModelAssetsError::DigestMismatch {
        model_id: model_id.to_owned(),
        role: role.to_owned(),
        expected: expected.to_owned(),
        actual,
    })
}

/// Lowercase hex encoding of raw bytes (no hashing).
fn hex_encode(bytes: impl AsRef<[u8]>) -> String {
    use std::fmt::Write as _;
    let mut out = String::with_capacity(64);
    for byte in bytes.as_ref() {
        let _ = write!(out, "{byte:02x}");
    }
    out
}

/// Hex-encoded SHA-256 of a byte slice.
pub fn hex_digest(bytes: &[u8]) -> String {
    hex_encode(Sha256::digest(bytes))
}

fn verify_file(model_id: &str, asset: &PinnedAsset, dir: &Path) -> Result<(), ModelAssetsError> {
    let path = dir.join(&asset.path);
    if !path.is_file() {
        return Err(ModelAssetsError::MissingAsset {
            role: asset.role.clone(),
            path,
        });
    }
    let metadata = fs::metadata(&path).map_err(|source| ModelAssetsError::Read {
        operation: AssetOperation::InspectMetadata,
        path: path.clone(),
        source,
    })?;
    if let Some(expected) = asset.size {
        if metadata.len() != expected {
            return Err(ModelAssetsError::SizeMismatch {
                path,
                expected,
                actual: metadata.len(),
            });
        }
    }
    let file = File::open(&path).map_err(|source| ModelAssetsError::Read {
        operation: AssetOperation::Open,
        path: path.clone(),
        source,
    })?;
    let mut reader = BufReader::with_capacity(BUFFER_SIZE, file);
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; BUFFER_SIZE];
    loop {
        let read = reader
            .read(&mut buffer)
            .map_err(|source| ModelAssetsError::Read {
                operation: AssetOperation::Stream,
                path: path.clone(),
                source,
            })?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    // `finalize()` is already SHA-256(file); re-hashing it would compare
    // SHA256(SHA256(file)) with the registry's SHA256(file) and reject every
    // correct asset.
    let actual = hex_encode(hasher.finalize());
    if actual == asset.sha256 {
        return Ok(());
    }
    Err(ModelAssetsError::DigestMismatch {
        model_id: model_id.to_owned(),
        role: asset.role.clone(),
        expected: asset.sha256.clone(),
        actual,
    })
}

/// Verify a local model directory against every asset pinned in the registry.
pub fn verify_dir(model_id: &str, dir: &Path) -> Result<(), ModelAssetsError> {
    let entry =
        model_entry(model_id).ok_or_else(|| ModelAssetsError::UnknownModel(model_id.to_owned()))?;
    for asset in &entry.assets {
        verify_file(model_id, asset, dir)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn registry_pins_full_digests_for_both_models() {
        assert_eq!(registry_version(), 2);
        for id in [DEFAULT_MODEL_ID, "fastino/gliner2-privacy-filter-PII-multi"] {
            let entry = model_entry(id).unwrap_or_else(|| panic!("{id} must be registered"));
            assert!(!entry.assets.is_empty());
            for asset in &entry.assets {
                assert_eq!(asset.sha256.len(), 64, "{}", asset.path);
                assert!(asset.sha256.bytes().all(|b| b.is_ascii_hexdigit()));
                assert!(!asset.role.is_empty());
            }
            let digests = digests_for(id).expect("three inference digests");
            for value in [&digests.model, &digests.tokenizer, &digests.encoder] {
                assert_eq!(value.len(), 64);
            }
        }
        assert!(!is_known_model("unknown/model"));
        assert!(digests_for("unknown/model").is_none());
    }

    #[test]
    fn fastino_entry_keeps_the_tauri_allowlist() {
        let entry = model_entry("fastino/gliner2-privacy-filter-PII-multi").unwrap();
        assert_eq!(
            entry.revision.as_deref(),
            Some("1cb4166094dc58fa8d836429f060d6c95f62b495")
        );
        assert_eq!(entry.assets.len(), 5);
        let model = entry.assets.iter().find(|a| a.role == "model").unwrap();
        assert_eq!(model.path, "model.safetensors");
        assert_eq!(model.size, Some(1_228_421_964));
        assert_eq!(
            model.sha256,
            "0280f6f39f6012da50b6640bad438d9b7e763a1b0102094115d1b710c4dd79b6"
        );
    }

    #[test]
    fn verify_bytes_accepts_matching_digest_and_rejects_change() {
        let bytes = b"model bytes";
        let digest = hex_digest(bytes);
        verify_bytes("m", "model", bytes, &digest).expect("matching digest");
        let err = verify_bytes("m", "model", bytes, &"0".repeat(64))
            .expect_err("changed digest must fail");
        assert!(matches!(err, ModelAssetsError::DigestMismatch { .. }));
    }

    /// Regression for the double-hash bug: `verify_file` must compare the
    /// file's own SHA-256 with the registry pin, not SHA256(SHA256(file)).
    #[test]
    fn verify_file_accepts_matching_pinned_digest() {
        let dir = std::env::temp_dir().join(format!(
            "hacienda-model-assets-verify-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).expect("temp model dir");
        let bytes = b"pinned model bytes";
        std::fs::write(dir.join("model.safetensors"), bytes).expect("write pinned asset");

        let asset = PinnedAsset {
            role: "model".to_string(),
            path: "model.safetensors".to_string(),
            sha256: hex_digest(bytes),
            size: Some(bytes.len() as u64),
        };
        verify_file("m", &asset, &dir).expect("a correct pinned file must verify");

        let mut changed = asset;
        changed.sha256 = "0".repeat(64);
        assert!(matches!(
            verify_file("m", &changed, &dir),
            Err(ModelAssetsError::DigestMismatch { .. })
        ));

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn verify_dir_reports_unknown_model_and_missing_asset() {
        let dir = std::env::temp_dir().join(format!(
            "hacienda-model-assets-missing-{}",
            std::process::id()
        ));
        let err = verify_dir("unknown/model", &dir).expect_err("unknown model must fail");
        assert!(matches!(err, ModelAssetsError::UnknownModel(_)));
        let err = verify_dir(DEFAULT_MODEL_ID, &dir).expect_err("missing file must fail");
        assert!(matches!(err, ModelAssetsError::MissingAsset { .. }));
    }

    #[test]
    fn verification_errors_carry_operation_path_source_and_recovery() {
        use std::error::Error as _;

        for (operation, label) in [
            (AssetOperation::InspectMetadata, "inspecting metadata"),
            (AssetOperation::Open, "opening"),
            (AssetOperation::Stream, "streaming"),
        ] {
            let err = ModelAssetsError::Read {
                operation,
                path: PathBuf::from("models/model.safetensors"),
                source: io::Error::new(io::ErrorKind::PermissionDenied, "permission denied"),
            };
            let rendered = err.to_string();
            assert!(rendered.contains(label), "{rendered}");
            assert!(rendered.contains("models/model.safetensors"), "{rendered}");
            assert!(rendered.contains("permission denied"), "{rendered}");
            assert!(
                rendered.contains("re-download the pinned asset"),
                "{rendered}"
            );
            assert!(err.source().is_some(), "the io::Error must stay reachable");
        }

        let contextual = [
            ModelAssetsError::UnknownModel("unknown/model".to_owned()),
            ModelAssetsError::MissingAsset {
                role: "model".to_owned(),
                path: PathBuf::from("model.safetensors"),
            },
            ModelAssetsError::SizeMismatch {
                path: PathBuf::from("model.safetensors"),
                expected: 10,
                actual: 9,
            },
            ModelAssetsError::DigestMismatch {
                model_id: "m".to_owned(),
                role: "model".to_owned(),
                expected: "a".repeat(64),
                actual: "b".repeat(64),
            },
        ];
        for err in contextual {
            assert!(
                err.to_string().contains("re-download"),
                "missing recovery suggestion: {err}"
            );
        }
    }

    #[test]
    fn build_validated_registry_has_unique_ids_and_relative_paths() {
        let registry = registry();
        let ids: HashSet<&str> = registry
            .models
            .iter()
            .map(|model| model.id.as_str())
            .collect();
        assert_eq!(ids.len(), registry.models.len(), "model ids must be unique");
        for model in &registry.models {
            assert!(
                !model.assets.is_empty(),
                "model '{}' has no assets",
                model.id
            );
            for asset in &model.assets {
                let path = Path::new(&asset.path);
                assert!(!path.is_absolute(), "{}", asset.path);
                assert!(
                    !path.components().any(|component| {
                        matches!(
                            component,
                            std::path::Component::ParentDir
                                | std::path::Component::RootDir
                                | std::path::Component::Prefix(_)
                        )
                    }),
                    "{} must not escape the model directory",
                    asset.path
                );
            }
        }
    }
}
