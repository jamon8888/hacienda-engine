//! hacienda-email: Email processing plugin for hacienda
//!
//! Provides email archiving, NER extraction, RAG indexing, and PII redaction
//! using the Pimalaya email ecosystem (io-email, io-imap, io-maildir, etc.).

pub mod archive;
pub mod attachments;
pub mod backend;
pub mod config;
pub mod error;
pub mod models;
pub mod ner;
pub mod pipeline;
pub mod rag;
pub mod redact;

use crate::backend::EmailClient;
use crate::config::EmailConfig;
use crate::error::{EmailError, Result};
use crate::models::{EmailOperation, EmailRequest, EmailResponse};
use std::sync::Arc;

/// The hacienda email processor
pub struct EmailProcessor {
    client: Arc<dyn EmailClient>,
    config: EmailConfig,
}

impl EmailProcessor {
    /// Create a new email processor
    pub async fn new(config: EmailConfig) -> Result<Self> {
        let client = crate::backend::build_client(&config).await?;
        Ok(Self { client, config })
    }

    /// Process an email request
    pub async fn process(&self, input: EmailRequest) -> Result<EmailResponse> {
        let config = input.config_overrides.as_ref().unwrap_or(&self.config);

        match input.operation {
            EmailOperation::Archive { .. } => self.archive(input.operation, config).await,
            EmailOperation::ExtractNER { .. } => self.extract_ner(input.operation, config).await,
            EmailOperation::IndexForRag { .. } => self.index_for_rag(input.operation, config).await,
            EmailOperation::Redact { .. } => self.redact(input.operation, config).await,
            EmailOperation::Pipeline { .. } => self.pipeline(input.operation, config).await,
        }
    }

    /// Archive emails
    async fn archive(&self, op: EmailOperation, config: &EmailConfig) -> Result<EmailResponse> {
        crate::archive::archive(self.client.as_ref(), op, config).await
    }

    /// Extract named entities
    async fn extract_ner(&self, op: EmailOperation, config: &EmailConfig) -> Result<EmailResponse> {
        crate::ner::extract_ner(self.client.as_ref(), op, config).await
    }

    /// Index for RAG
    async fn index_for_rag(&self, op: EmailOperation, config: &EmailConfig) -> Result<EmailResponse> {
        crate::rag::index_for_rag(self.client.as_ref(), op, config).await
    }

    /// Redact PII
    async fn redact(&self, op: EmailOperation, config: &EmailConfig) -> Result<EmailResponse> {
        crate::redact::redact(self.client.as_ref(), op, config).await
    }

    /// Full pipeline
    async fn pipeline(&self, op: EmailOperation, config: &EmailConfig) -> Result<EmailResponse> {
        crate::pipeline::run_pipeline(self.client.as_ref(), op, config).await
    }
}

/// Re-export for convenience
pub use crate::config::EmailConfig;
pub use crate::models::{EmailRequest, EmailResponse, EmailOperation, EmailSource, EmailDestination, PipelineStep};
pub use crate::error::EmailError;