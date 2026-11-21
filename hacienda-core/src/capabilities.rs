//! Shared, safe capability contract used by Hacienda's public surfaces.

use serde::Serialize;
use std::sync::OnceLock;

#[cfg(all(
    feature = "rag-embeddings",
    target_os = "macos",
    target_arch = "x86_64"
))]
use std::path::Path;

/// Build- and runtime-readiness of one safe Hacienda capability.
#[derive(Debug, Clone, Serialize)]
pub struct CapabilityStatus {
    pub name: &'static str,
    pub compiled: bool,
    pub runtime_ready: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<&'static str>,
}

/// Stable capability report shared by REST, CLI, and MCP.
#[derive(Debug, Clone, Serialize)]
pub struct CapabilityReport {
    pub version: &'static str,
    pub capabilities: Vec<CapabilityStatus>,
    pub embedding_mode: &'static str,
    pub onnx_runtime: CapabilityStatus,
    /// Readiness of the model-backed NER half of PII detection.
    pub ner_model: CapabilityStatus,
    /// Pinned id of the NER model that actually loaded, `None` for regex-only.
    pub ner_model_id: Option<String>,
}

/// Compose the common capability report from surface-specific runtime checks.
pub fn report(
    version: &'static str,
    embedding_compiled: bool,
    onnx_runtime: CapabilityStatus,
    service_ready: bool,
) -> CapabilityReport {
    let embedding_migration = CapabilityStatus {
        name: "embedding_migration",
        compiled: embedding_compiled,
        runtime_ready: embedding_compiled && onnx_runtime.runtime_ready && service_ready,
        reason: if !embedding_compiled {
            Some("rebuild with the rag-embeddings feature to enable real re-embedding")
        } else if !onnx_runtime.runtime_ready {
            onnx_runtime.reason
        } else if !service_ready {
            Some("configure a RAG store before starting embedding migration jobs")
        } else {
            None
        },
    };

    let ner_model_id = crate::pii::ner::loaded_model_id().map(str::to_owned);
    let ner_model_compiled = cfg!(any(
        all(feature = "ner-candle", not(target_arch = "wasm32")),
        all(feature = "ner-candle-wasm", target_arch = "wasm32"),
    ));
    let ner_model = CapabilityStatus {
        name: "ner_model",
        compiled: ner_model_compiled,
        runtime_ready: ner_model_id.is_some(),
        reason: if ner_model_id.is_some() {
            None
        } else if ner_model_compiled {
            Some("regex-only: configure [pii.model] model_id and model_dir to load GLiNER2")
        } else {
            Some("rebuild with ner-candle (or ner-candle-wasm) to enable model-backed NER")
        },
    };

    CapabilityReport {
        version,
        capabilities: vec![
            CapabilityStatus {
                name: "redaction",
                compiled: true,
                runtime_ready: true,
                reason: None,
            },
            CapabilityStatus {
                name: "document_extraction",
                compiled: true,
                runtime_ready: true,
                reason: None,
            },
            CapabilityStatus {
                name: "embedding_presets",
                compiled: true,
                runtime_ready: true,
                reason: None,
            },
            embedding_migration,
            ner_model.clone(),
        ],
        embedding_mode: if embedding_compiled {
            "real"
        } else {
            "presets_only"
        },
        onnx_runtime,
        ner_model,
        ner_model_id,
    }
}

/// Probe the ONNX runtime prerequisite without exposing paths or loading document data.
pub fn onnx_runtime_status() -> CapabilityStatus {
    #[cfg(feature = "rag-embeddings")]
    {
        #[cfg(all(target_os = "macos", target_arch = "x86_64"))]
        {
            let path = std::env::var_os("ORT_DYLIB_PATH")
                .map(Path::new)
                .unwrap_or_else(|| Path::new("libonnxruntime.dylib"));
            let ready = can_load_onnx_runtime(path);
            return CapabilityStatus {
                name: "onnx_runtime",
                compiled: true,
                runtime_ready: ready,
                reason: if ready {
                    None
                } else {
                    Some("set ORT_DYLIB_PATH to a loadable libonnxruntime.dylib")
                },
            };
        }

        return CapabilityStatus {
            name: "onnx_runtime",
            compiled: true,
            runtime_ready: true,
            reason: None,
        };
    }

    #[allow(unreachable_code)]
    CapabilityStatus {
        name: "onnx_runtime",
        compiled: false,
        runtime_ready: false,
        reason: Some("not compiled; enable the rag-embeddings feature for ONNX re-embedding"),
    }
}

/// Probe the ONNX runtime once per process so capability requests do not repeatedly load FFI.
pub fn cached_onnx_runtime_status() -> CapabilityStatus {
    static STATUS: OnceLock<CapabilityStatus> = OnceLock::new();
    STATUS.get_or_init(onnx_runtime_status).clone()
}

#[cfg(all(
    feature = "rag-embeddings",
    target_os = "macos",
    target_arch = "x86_64"
))]
fn can_load_onnx_runtime(path: &Path) -> bool {
    let Ok(library) = (unsafe { libloading::Library::new(path) }) else {
        return false;
    };

    // SAFETY: the symbol type matches ONNX Runtime's public ABI, and every pointer is
    // checked before it is dereferenced. The probe calls only version/query functions
    // while the library remains loaded; it does not retain any foreign pointer afterward.
    unsafe {
        let Ok(base_getter) =
            library.get::<unsafe extern "C" fn() -> *const ort_sys::OrtApiBase>(b"OrtGetApiBase\0")
        else {
            return false;
        };
        let Some(base) = std::ptr::NonNull::new(base_getter().cast_mut()) else {
            return false;
        };
        if ((*base.as_ptr()).GetVersionString as usize) == 0
            || ((*base.as_ptr()).GetApi as usize) == 0
        {
            return false;
        }
        let version = ((*base.as_ptr()).GetVersionString)();
        if version.is_null() {
            return false;
        }
        let api = ((*base.as_ptr()).GetApi)(ort_sys::ORT_API_VERSION);
        !api.is_null()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn report_keeps_embedding_migration_unavailable_without_service() {
        let report = report(
            "test",
            true,
            CapabilityStatus {
                name: "onnx_runtime",
                compiled: true,
                runtime_ready: true,
                reason: None,
            },
            false,
        );

        assert_eq!(report.embedding_mode, "real");
        assert!(!report.capabilities[3].runtime_ready);
        assert_eq!(
            report.capabilities[3].reason,
            Some("configure a RAG store before starting embedding migration jobs")
        );
    }

    #[test]
    fn report_advertises_ner_model_readiness_from_loaded_state() {
        let report = report(
            "test",
            false,
            CapabilityStatus {
                name: "onnx_runtime",
                compiled: false,
                runtime_ready: false,
                reason: None,
            },
            true,
        );
        let status = report
            .capabilities
            .iter()
            .find(|capability| capability.name == "ner_model")
            .expect("ner_model capability must be reported");
        assert_eq!(status.name, "ner_model");
        // No model has been loaded in this test process: regex-only, reported
        // honestly instead of echoing a configuration that may never have worked.
        assert!(!status.runtime_ready);
        assert!(report.ner_model_id.is_none());
    }
}
