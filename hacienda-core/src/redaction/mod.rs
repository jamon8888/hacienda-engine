//! Span redaction with a per-result blake3 hash chain.

/// Module engine
pub mod engine;
/// Module pseudonym
pub mod pseudonym;
/// Module types
pub mod types;

pub use engine::RedactionEngine;
pub use pseudonym::{
    EnvKeyResolver, KeyId, KeyResolver, KeyStatus, PseudonymError, PseudonymKey, Pseudonymiser,
    ACTIVE_KEY_VAR, KEY_BYTES, KEY_VAR_PREFIX,
};
pub use types::{
    RedactionAuditEntry, RedactionConfig, RedactionMetrics, RedactionMode, RedactionResult,
};

use thiserror::Error;

#[derive(Debug, Error)]
/// RedactionError enum
pub enum RedactionError {
    #[error(
        "unknown redaction mode: '{0}' (expected mask, hash, pseudonymize, remove, or custom)"
    )]
    UnknownMode(String),

    /// Refuses to build a `Pseudonymize` engine with no key.
    ///
    /// Deliberately not a fallback to masking. Masking and pseudonymisation are different
    /// controls with different legal consequences (GDPR Art. 4(5)), and quietly applying
    /// the weaker one would leave an operator believing they hold reversible output when
    /// the value is gone for good.
    #[error(
        "redaction mode 'pseudonymize' requires a pseudonymisation key, and none was supplied \
         (set {ACTIVE_KEY_VAR}, or choose mode 'mask')"
    )]
    MissingPseudonymKey,

    #[error(transparent)]
    Pseudonym(#[from] PseudonymError),
}
