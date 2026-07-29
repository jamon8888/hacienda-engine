//! Shared application state threaded through every handler.

use hacienda_core::jobs::JobStore;
use hacienda_core::{auth::AuthState, HaciendaFacade};
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
        }
    }
}
