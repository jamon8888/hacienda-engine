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
//! [`ApiState`] wraps the injected [`JobStore`] in [`GuardedJobStore`] to enforce
//! bounded retention and a disabled retry policy. Durability is a property of the
//! injected backend only: the in-repo `hacienda serve` injects an in-memory store and
//! **loses queued jobs on restart**; a host that builds and migrates a Postgres pool
//! can inject `PostgresJobStore` via [`ApiState::with_job_store`].
//!
//! [`AuthState`]: hacienda_core::auth::AuthState
//! [`JobStore`]: hacienda_core::jobs::JobStore
//! [`Router`]: axum::Router

pub mod dto;
mod error;
mod extract;
mod handlers;
pub mod job_state;
mod quota;
mod routes;
mod state;
/// TeamOnPremise email integration: source_ref/mailbox_ref ingestion, job/RAG polling and
/// tenant/document/email-bound viewer capabilities. See the module docs for the transport
/// boundary: no path, credential or raw message text is accepted from a renderer.
pub mod team_email;

pub use error::{ApiError, ApiErrorCode};
pub use hacienda_client_core::ProviderSseEvent;
pub use job_state::{GuardedJobStore, JobRetention, RetryPolicy};
pub use handlers::claude_messages::{
    AnthropicMessagesProvider, ClaudeMessagesEventStream, ClaudeMessagesProvider,
    ClaudeMessagesProviderRequest, ClaudeMessagesProviderResponse, ClaudeMessagesProviderStream,
    ProviderGatewayError,
};
pub use quota::{InMemoryQuotaStore, QuotaLimits};
pub use state::{ApiLimits, ApiState};

/// Build the axum router for hacienda.
///
/// Both route registration and auth requirements are derived from the single route table
/// in [`routes::ROUTE_TABLE`]. The body limit is applied to every route.
pub fn router(state: ApiState) -> axum::Router {
    routes::build_router(state)
}
