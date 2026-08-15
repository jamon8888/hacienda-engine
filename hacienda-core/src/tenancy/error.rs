//! Error type for tenant store operations.

use super::TenantId;
use thiserror::Error;

/// Errors returned by [`super::store::TenantStore`] implementations.
#[derive(Debug, Error)]
pub enum TenantError {
    /// `create` was called with an id that already has a row.
    ///
    /// Not a race-tolerant compare-and-swap error like [`crate::jobs::JobError::
    /// StatusMismatch`] — tenant admission is rare, operator-driven, and idempotent
    /// re-admission (spec §8's migration) is the caller's job to detect via `get`
    /// first, not this store's to silently allow. Silently allowing it would let a
    /// second `create("acme", ...)` with a different `display_name` overwrite the
    /// first without the caller ever deciding that was intended.
    #[error("tenant already exists: {0}")]
    AlreadyExists(TenantId),

    /// No tenant with the given id exists in the store.
    #[error("tenant not found: {0}")]
    NotFound(TenantId),

    /// An internal store error (e.g. lock poisoning) that cannot be recovered by the
    /// caller. Should not occur under normal operation.
    #[error("tenant store internal error: {0}")]
    Internal(String),
}

/// Lets Postgres backend code use `?` directly on `sqlx::Error`. Gated behind the
/// `postgres` feature so non-Postgres consumers never pull in `sqlx` for this impl.
#[cfg(feature = "postgres")]
impl From<sqlx::Error> for TenantError {
    fn from(e: sqlx::Error) -> Self {
        TenantError::Internal(e.to_string())
    }
}
