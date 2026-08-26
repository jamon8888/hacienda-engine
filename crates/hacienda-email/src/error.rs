use thiserror::Error;

/// Errors that can occur during email processing
#[derive(Debug, Error)]
pub enum EmailError {
    #[error("Configuration error: {0}")]
    Config(String),

    #[error("Backend error: {0}")]
    Backend(String),

    #[error("Connection error: {0}")]
    Connection(String),

    #[error("Authentication error: {0}")]
    Auth(String),

    #[error("Mailbox not found: {0}")]
    MailboxNotFound(String),

    #[error("Message not found: {0}")]
    MessageNotFound(String),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Serialization error: {0}")]
    Serde(#[from] serde_json::Error),

    #[error("URL parse error: {0}")]
    UrlParse(#[from] url::ParseError),

    #[error("NER error: {0}")]
    Ner(String),

    #[error("RAG indexing error: {0}")]
    Rag(String),

    #[error("Redaction error: {0}")]
    Redaction(String),

    #[error("Attachment extraction error: {0}")]
    Attachment(String),

    #[error("Plugin error: {0}")]
    Plugin(String),

    #[error("Invalid operation: {0}")]
    InvalidOperation(String),
}

pub type Result<T> = std::result::Result<T, EmailError>;
