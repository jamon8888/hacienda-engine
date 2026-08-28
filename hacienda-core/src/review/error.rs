use thiserror::Error;

#[derive(Debug, Error)]
/// ReviewError enum
pub enum ReviewError {
    #[error("review item not found: {0}")]
    /// NotFound variant
    NotFound(String),

    #[error("review item already decided: {0}")]
    /// AlreadyDecided variant
    AlreadyDecided(String),

    #[error("invalid review status transition from {from} to {to}")]
    InvalidTransition { from: String, to: String },

    /// Returned by [`FileReviewStore`] when a file operation fails.
    ///
    /// [`FileReviewStore`]: crate::review::store_file::FileReviewStore
    #[error("review store I/O error at {path}: {source}")]
    Io {
        path: String,
        #[source]
        source: std::io::Error,
    },

    /// Returned by [`FileReviewStore`] when a log line cannot be parsed.
    ///
    /// A mid-file malformed line is an error; only the trailing line (a crash mid-append)
    /// is dropped silently. See [`FileReviewStore`]'s module documentation.
    ///
    /// [`FileReviewStore`]: crate::review::store_file::FileReviewStore
    #[error("review store JSON error: {0}")]
    Json(#[from] serde_json::Error),

    /// An internal store error (e.g. a database driver failure) that cannot be
    /// recovered by the caller. Mirrors `JobError::Internal`'s convention for the
    /// same kind of failure in the sibling job store.
    #[error("review store internal error: {0}")]
    Internal(String),
}

/// Lets Postgres backend code use `?` directly on `sqlx::Error`. Gated behind the
/// `postgres` feature so non-Postgres consumers never pull in `sqlx` for this impl.
#[cfg(feature = "postgres")]
impl From<sqlx::Error> for ReviewError {
    fn from(e: sqlx::Error) -> Self {
        ReviewError::Internal(e.to_string())
    }
}
