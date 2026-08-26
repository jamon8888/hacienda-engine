use crate::error::{EmailError, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Email processing operations
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case")]
pub enum EmailOperation {
    /// Archive emails from source to destination
    Archive {
        source: EmailSource,
        destination: EmailDestination,
        #[serde(skip_serializing_if = "Option::is_none")]
        since: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        before: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        mailbox: Option<String>,
        include_attachments: bool,
    },

    /// Extract named entities from emails
    ExtractNER {
        source: EmailSource,
        output: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        model: Option<String>,
    },

    /// Index emails for RAG
    IndexForRag {
        source: EmailSource,
        #[serde(skip_serializing_if = "Option::is_none")]
        index_name: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        chunk_size: Option<usize>,
    },

    /// Redact PII from emails
    Redact {
        source: EmailSource,
        destination: EmailDestination,
        #[serde(skip_serializing_if = "Option::is_none")]
        rules: Option<Vec<RedactionRule>>,
    },

    /// Full pipeline: archive -> extract attachments -> NER -> RAG index
    Pipeline {
        source: EmailSource,
        destination: EmailDestination,
        steps: Vec<PipelineStep>,
    },
}

/// Email source specification
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum EmailSource {
    #[serde(rename = "account")]
    Account { name: String, mailbox: Option<String> },

    #[serde(rename = "maildir")]
    Maildir { path: String },

    #[serde(rename = "files")]
    Files { paths: Vec<String> },

    #[serde(rename = "stdin")]
    Stdin,
}

/// Email destination specification
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum EmailDestination {
    #[serde(rename = "maildir")]
    Maildir { path: String },

    #[serde(rename = "files")]
    Files { path: String },

    #[serde(rename = "stdout")]
    Stdout,
}

/// Pipeline steps
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PipelineStep {
    Archive,
    ExtractAttachments,
    Ner,
    RagIndex,
    Redact,
}

/// Plugin request
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmailRequest {
    pub operation: EmailOperation,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub config_overrides: Option<crate::config::EmailConfig>,
}

/// Plugin response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmailResponse {
    pub success: bool,
    pub processed_count: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entities: Option<Vec<ExtractedEntity>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rag_stats: Option<RagIndexStats>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub redaction_stats: Option<RedactionStats>,
    pub errors: Vec<String>,
}

impl Default for EmailResponse {
    fn default() -> Self {
        Self {
            success: false,
            processed_count: 0,
            output_path: None,
            entities: None,
            rag_stats: None,
            redaction_stats: None,
            errors: Vec::new(),
        }
    }
}

/// Extracted entity from NER
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtractedEntity {
    pub text: String,
    pub label: String,
    pub confidence: f32,
    pub email_id: String,
    pub span: (usize, usize),
}

/// RAG indexing statistics
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RagIndexStats {
    pub chunks_created: usize,
    pub vectors_indexed: usize,
    pub index_name: String,
}

/// Redaction statistics
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RedactionStats {
    pub total_redactions: usize,
    pub by_type: HashMap<String, usize>,
}

/// Extracted attachment
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtractedAttachment {
    pub filename: String,
    pub mime_type: String,
    pub content: String,
    pub metadata: serde_json::Value,
}

/// Email message (simplified for internal use)
#[derive(Debug, Clone)]
pub struct EmailMessage {
    pub id: String,
    pub mailbox: String,
    pub subject: String,
    pub from: Vec<EmailAddress>,
    pub to: Vec<EmailAddress>,
    pub cc: Vec<EmailAddress>,
    pub bcc: Vec<EmailAddress>,
    pub date: chrono::DateTime<chrono::Utc>,
    pub text_body: Option<String>,
    pub html_body: Option<String>,
    pub attachments: Vec<EmailAttachment>,
    pub flags: Vec<String>,
}

/// Email address
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmailAddress {
    pub name: Option<String>,
    pub address: String,
}

/// Email attachment
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmailAttachment {
    pub filename: String,
    pub content_type: String,
    pub size: usize,
    pub content_id: Option<String>,
}

/// Mailbox information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Mailbox {
    pub name: String,
    pub total: Option<usize>,
    pub unread: Option<usize>,
    pub flags: Vec<Flag>,
}

/// Email envelope (summary)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Envelope {
    pub id: String,
    pub mailbox: String,
    pub subject: String,
    pub from: Vec<EmailAddress>,
    pub to: Vec<EmailAddress>,
    pub date: chrono::DateTime<chrono::Utc>,
    pub flags: Vec<Flag>,
    pub has_attachments: bool,
}

/// Email flags
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Flag {
    Seen,
    Answered,
    Flagged,
    Deleted,
    Draft,
    Recent,
    Custom(String),
}

/// Redaction rule (re-exported for public API)
pub use crate::config::RedactionRule;
