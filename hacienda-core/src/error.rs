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
}

impl From<xberg::XbergError> for HaciendaError {
    fn from(source: xberg::XbergError) -> Self {
        HaciendaError::Extraction(Box::new(source))
    }
}
