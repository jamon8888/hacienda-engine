use thiserror::Error;

#[derive(Debug, Error)]
pub enum ReviewError {
    #[error("review item not found: {0}")]
    NotFound(String),

    #[error("review item already decided: {0}")]
    AlreadyDecided(String),

    #[error("invalid review status transition from {from} to {to}")]
    InvalidTransition { from: String, to: String },
}
