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
        ],
        embedding_mode: if embedding_compiled {
            "real"
        } else {
            "presets_only"
        },
        onnx_runtime,
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
}
