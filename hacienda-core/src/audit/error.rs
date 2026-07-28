use thiserror::Error;

#[derive(Debug, Error)]
pub enum AuditError {
    #[error("writing audit log to {path}: {source}")]
    Io {
        path: String,
        #[source]
        source: std::io::Error,
    },

    #[error("serializing audit entry: {0}")]
    Json(#[from] serde_json::Error),

    #[error("audit chain broken at entry {index}: expected hash '{expected}', found '{actual}'")]
    ChainIntegrity {
        index: u64,
        expected: String,
        actual: String,
    },

    #[error("audit entry was minted under config '{actual}' but this chain uses '{expected}'")]
    ConfigMismatch { expected: String, actual: String },
}
