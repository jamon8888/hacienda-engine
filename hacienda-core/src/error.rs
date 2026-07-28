use thiserror::Error;

#[derive(Debug, Error)]
pub enum HaciendaError {
    #[error("extraction failed: {0}")]
    Extraction(String),

    #[error("pii pipeline failed: {0}")]
    Pii(String),

    #[error("compliance failed: {0}")]
    Compliance(String),

    #[error("audit failed: {0}")]
    Audit(String),

    #[error("review failed: {0}")]
    Review(String),

    #[error("glossary failed: {0}")]
    Glossary(String),

    #[error("configuration error: {0}")]
    Config(String),
}

impl From<xberg::XbergError> for HaciendaError {
    fn from(e: xberg::XbergError) -> Self {
        HaciendaError::Extraction(e.to_string())
    }
}
