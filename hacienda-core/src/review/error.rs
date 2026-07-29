use thiserror::Error;

#[derive(Debug, Error)]
pub enum ReviewError {
    #[error("review item not found: {0}")]
    NotFound(String),

    #[error("review item already decided: {0}")]
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
}
