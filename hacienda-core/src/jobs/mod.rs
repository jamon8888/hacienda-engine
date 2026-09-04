//! Async job tracking for document-processing pipelines.
//!
//! Provides the vocabulary and in-memory store that Phase 4's HTTP endpoints
//! (`POST /v1/documents/async`, `GET /v1/jobs/{id}`) will build on. The module
//! is deliberately thin: no file backend, no priorities, no retries, no TTLs —
//! none of those are specified and adding them now would be speculation ahead of
//! a real consumer. See plan D5.

/// Module error
pub mod error;
/// Module store
pub mod store;
/// Module types
pub mod types;

pub use error::JobError;
pub use store::{InMemoryJobStore, JobStore};
pub use types::{Job, JobStatus};
