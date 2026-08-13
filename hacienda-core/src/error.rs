//! The single error type crossing the [`crate::HaciendaFacade`] boundary.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum HaciendaError {
    /// Document extraction failed inside xberg.
    #[error("extraction failed")]
    Extraction(#[source] Box<xberg::XbergError>),

    #[error(transparent)]
    Pii(#[from] crate::pii::PiiError),

    #[error(transparent)]
    Audit(#[from] crate::audit::AuditError),

    #[error(transparent)]
    Review(#[from] crate::review::ReviewError),

    #[error(transparent)]
    Authz(#[from] crate::auth::AuthzError),

    /// A pseudonym token operation failed.
    ///
    /// This variant exists so that `PseudonymError` can convert directly to
    /// `HaciendaError` without requiring the caller to manually map through
    /// `RedactionError` and `PiiError`. The underlying error is preserved for
    /// diagnostics while the HTTP layer can collapse all token errors to a
    /// single 400 response.
    #[error(transparent)]
    Pseudonym(#[from] crate::redaction::PseudonymError),

    /// An API key operation failed.
    #[error(transparent)]
    ApiKey(#[from] crate::auth::keys::ApiKeyError),

    /// A detection or redaction operation was requested but no `[pii]` section is
    /// configured, so there is no pipeline to run.
    ///
    /// This is an error rather than an empty result on purpose. Reporting "no PII found"
    /// because the detector was never enabled is indistinguishable, to the caller, from
    /// a clean document — and it is the failure mode that gets privileged material
    /// disclosed.
    #[error("PII detection is not enabled: add a [pii] section to the configuration")]
    PiiDisabled,

    /// Structured-field redaction recursed into a nested archive member deeper than
    /// [`crate::facade::MAX_REDACTION_DEPTH`] — defense-in-depth alongside xberg's own
    /// `ExtractionConfig::max_archive_depth`. See that constant's doc comment.
    #[error("archive nesting exceeds hacienda's redaction depth limit ({limit})")]
    RedactionDepthExceeded { limit: usize },
}

impl From<xberg::XbergError> for HaciendaError {
    fn from(source: xberg::XbergError) -> Self {
        HaciendaError::Extraction(Box::new(source))
    }
}
