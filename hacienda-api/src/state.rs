//! Shared application state threaded through every handler.

use hacienda_core::jobs::JobStore;
use hacienda_core::store::object::ObjectStore;
use hacienda_core::store::postgres::presets::PresetStore;
use hacienda_core::store::postgres::usage::UsageStore;
use hacienda_core::store::postgres::versions::DocumentVersionStore;
use hacienda_core::{auth::AuthState, HaciendaFacade};
use hacienda_rag::RagStore;
use std::sync::Arc;

/// Body / batch limits applied uniformly to every request.
///
/// The defaults — 32 MiB for a body, 64 documents per batch — match the brief and
/// should be sufficient for French legal documents. Both limits are checked before
/// any decoding begins, so an oversized request is rejected cheaply.
#[derive(Debug, Clone)]
pub struct ApiLimits {
    /// Maximum accepted Content-Length (and body read limit) in bytes.
    ///
    /// Bodies that exceed this are rejected with 413 before the body is read.
    pub max_body_bytes: usize,
    /// Maximum number of documents in a single `POST /v1/documents` request.
    pub max_documents: usize,
}

impl Default for ApiLimits {
    fn default() -> Self {
        Self {
            max_body_bytes: 32 * 1024 * 1024, // 32 MiB
            max_documents: 64,
        }
    }
}

/// State shared by every request handler.
///
/// Clone is cheap — every field is either `Arc` or `Copy`.
#[derive(Clone)]
pub struct ApiState {
    /// The business-logic facade. All document processing, scanning, and audit go here.
    pub(crate) facade: Arc<HaciendaFacade>,
    /// Async job registry. In-memory only; dies with the process.
    pub(crate) jobs: Arc<dyn JobStore>,
    /// Auth middleware configuration. Owns the token resolver and route requirements.
    pub(crate) auth: AuthState,
    /// Per-request resource limits.
    pub(crate) limits: ApiLimits,
    /// RAG vector store backing `/v1/rag/*`. `None` means those routes are disabled
    /// (`ApiError::invalid_request`, mirroring how a `None` PII config disables
    /// `/v1/pii/*`'s pipeline-dependent behaviour) — RAG is opt-in, not every embedder
    /// wants the dependency or a live collection store.
    pub(crate) rag_store: Option<Arc<dyn RagStore>>,
    /// Preset store backing `/v1/presets/*`. `None` means those routes are disabled
    /// (`ApiError::invalid_request`) — unlike `RagStore`/`JobStore`, `PresetStore`
    /// has no in-memory implementation (Postgres is its only backend), so this is
    /// opt-in the same way RAG is: attach it only when a pool is available.
    pub(crate) preset_store: Option<Arc<dyn PresetStore>>,
    /// Document version store backing `/v1/documents/{id}/versions`, `/v1/documents/{id}`,
    /// and the `/diff` routes. `None` means those routes are disabled
    /// (`ApiError::invalid_request`) — like `PresetStore`, `DocumentVersionStore` has no
    /// in-memory backend (Postgres-only), so this is opt-in the same way.
    pub(crate) version_store: Option<Arc<dyn DocumentVersionStore>>,
    /// Object store backing `/v1/uploads/presign` and `/v1/uploads/confirm`. `None` means
    /// those routes are disabled (`ApiError::invalid_request`) — like `PresetStore` and
    /// `DocumentVersionStore`, `ObjectStore`'s only implementation (`S3ObjectStore`) needs
    /// a live bucket, so this is opt-in the same way.
    pub(crate) object_store: Option<Arc<dyn ObjectStore>>,
    /// Usage read-model backing `GET /v1/usage`. `None` means that route is disabled
    /// (`ApiError::invalid_request`) — like `PresetStore`/`DocumentVersionStore`,
    /// `UsageStore`'s only implementation queries `audit_entries` directly (Postgres-only),
    /// so this is opt-in the same way.
    pub(crate) usage_store: Option<Arc<dyn UsageStore>>,
}

impl ApiState {
    pub fn new(
        facade: Arc<HaciendaFacade>,
        jobs: Arc<dyn JobStore>,
        auth: AuthState,
        limits: ApiLimits,
    ) -> Self {
        Self {
            facade,
            jobs,
            auth,
            limits,
            rag_store: None,
            preset_store: None,
            version_store: None,
            object_store: None,
            usage_store: None,
        }
    }

    /// Enable `/v1/rag/*` by attaching a store. Builder-style so existing call sites
    /// (which construct `ApiState` without RAG) do not need to change.
    #[must_use]
    pub fn with_rag_store(mut self, rag_store: Arc<dyn RagStore>) -> Self {
        self.rag_store = Some(rag_store);
        self
    }

    /// Enable `/v1/presets/*` by attaching a store. Builder-style, mirroring
    /// `with_rag_store` exactly, so existing call sites are unaffected.
    #[must_use]
    pub fn with_preset_store(mut self, preset_store: Arc<dyn PresetStore>) -> Self {
        self.preset_store = Some(preset_store);
        self
    }

    /// Enable `/v1/documents/{id}/versions`, `/v1/documents/{id}`, and the `/diff` routes
    /// by attaching a store. Builder-style, mirroring `with_preset_store` exactly, so
    /// existing call sites are unaffected.
    #[must_use]
    pub fn with_version_store(mut self, version_store: Arc<dyn DocumentVersionStore>) -> Self {
        self.version_store = Some(version_store);
        self
    }

    /// Enable `/v1/uploads/presign` and `/v1/uploads/confirm` by attaching a store.
    /// Builder-style, mirroring `with_version_store` exactly, so existing call sites are
    /// unaffected.
    #[must_use]
    pub fn with_object_store(mut self, object_store: Arc<dyn ObjectStore>) -> Self {
        self.object_store = Some(object_store);
        self
    }

    /// Enable `GET /v1/usage` by attaching a store. Builder-style, mirroring
    /// `with_object_store` exactly, so existing call sites are unaffected.
    #[must_use]
    pub fn with_usage_store(mut self, usage_store: Arc<dyn UsageStore>) -> Self {
        self.usage_store = Some(usage_store);
        self
    }
}
