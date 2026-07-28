//! Span redaction with a per-result blake3 hash chain.

pub mod engine;
pub mod types;

pub use engine::RedactionEngine;
pub use types::{
    RedactionAuditEntry, RedactionConfig, RedactionMetrics, RedactionMode, RedactionResult,
};

use thiserror::Error;

#[derive(Debug, Error)]
pub enum RedactionError {
    #[error(
        "unknown redaction mode: '{0}' (expected mask, hash, pseudonymize, remove, or custom)"
    )]
    UnknownMode(String),
}
