//! HTTP API for hacienda — axum router over [`HaciendaFacade`].
//!
//! # Design invariant: one route table
//!
//! Every route is declared in [`routes::ROUTE_TABLE`]. Both the axum [`Router`] and the
//! [`AuthState`] are derived from that table; it is structurally impossible to add a route
//! without deciding who may call it. See [`router`] and the test
//! `every_guarded_route_reflected_in_auth_state`.
//!
//! # Security
//!
//! This crate never accepts [`xberg::ExtractInput`] from the wire, because that type's
//! `Uri` variant accepts filesystem paths and HTTP URLs — local file read and SSRF.
//! All document input arrives as inline base64-encoded bytes; see [`dto::DocumentInput`].
//!
//! Error responses carry a generic message. Detail goes to [`tracing::error!`] on the
//! host. No handler may echo document content, filenames, or detected spans into an
//! error body.
//!
//! # Async jobs
//!
//! `POST /v1/documents/async` creates a job record and spawns a detached tokio task.
//! Jobs are held in an in-memory [`JobStore`] and **die with the process**. A durable
//! backend is Phase 6.
//!
//! [`AuthState`]: hacienda_core::auth::AuthState
//! [`JobStore`]: hacienda_core::jobs::JobStore
//! [`Router`]: axum::Router

mod dto;
mod error;
mod extract;
mod handlers;
mod quota;
mod routes;
mod state;

pub use error::{ApiError, ApiErrorCode};
pub use quota::{InMemoryQuotaStore, QuotaLimits};
pub use state::{ApiLimits, ApiState};

/// Build the axum router for hacienda.
///
/// Both route registration and auth requirements are derived from the single route table
/// in [`routes::ROUTE_TABLE`]. The body limit is applied to every route.
pub fn router(state: ApiState) -> axum::Router {
    routes::build_router(state)
}
