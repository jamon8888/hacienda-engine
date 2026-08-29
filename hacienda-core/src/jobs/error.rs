//! Error type for job store operations.

use super::types::JobStatus;
use thiserror::Error;

/// Errors returned by [`super::store::JobStore`] implementations.
///
/// Variants carry the job id where applicable so callers can log a useful
/// message without re-fetching the job.
#[derive(Debug, Error)]
/// JobError enum
pub enum JobError {
    /// No job with the given id exists in the store.
    #[error("job not found: {0}")]
    /// NotFound variant
    NotFound(String),

    /// A `transition` call failed because the job's current status was not the
    /// expected `from` value. This is the normal outcome for the losing worker in a
    /// two-worker race; callers should treat it as a signal to back off, not a bug.
    #[error("job {id}: expected status {expected} for transition, found {actual}")]
    StatusMismatch {
        /// Job ID
        id: String,
        /// Expected status
        expected: JobStatus,
        /// Actual status
        actual: JobStatus,
    },

    /// An internal store error (e.g. lock poisoning) that cannot be recovered by the
    /// caller. Should not occur under normal operation.
    #[error("job store internal error: {0}")]
    Internal(String),
}

/// Lets Postgres backend code use `?` directly on `sqlx::Error`. Gated behind the
/// `postgres` feature so non-Postgres consumers never pull in `sqlx` for this impl.
#[cfg(feature = "postgres")]
impl From<sqlx::Error> for JobError {
    fn from(e: sqlx::Error) -> Self {
        JobError::Internal(e.to_string())
    }
}
