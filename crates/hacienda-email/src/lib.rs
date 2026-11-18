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
pub mod pii;
pub mod pipeline;
pub mod rag;
pub mod redact;

use crate::backend::EmailClient;
use std::sync::Arc;

// Public API re-exports. A `pub use` also brings the name into this module's
// scope, so no separate private `use` is needed (doing both is E0252).
pub use crate::config::{EmailConfig, OAuthProvider, PrivacyMode};
pub use crate::error::{EmailError, Result};
pub use crate::models::{
    EmailDestination, EmailOperation, EmailRequest, EmailResponse, EmailSource, PipelineStep,
};

/// The hacienda email processor
pub struct EmailProcessor {
    client: Arc<dyn EmailClient>,
    config: EmailConfig,
    /// The real hacienda PII engine, shared by redaction and NER operations.
    pii: crate::pii::PiiEngine,
}

impl EmailProcessor {
    /// Create a new email processor
    pub async fn new(config: EmailConfig) -> Result<Self> {
        let client = crate::backend::build_client(&config).await?;
        let pii = crate::pii::PiiEngine::new(config.redaction.mode)?;
        Ok(Self { client, config, pii })
    }

    /// Process an email request
    pub async fn process(&self, input: EmailRequest) -> Result<EmailResponse> {
        let config = input.config_overrides.as_ref().unwrap_or(&self.config);

        // Per-tenant privacy mode (design D1): if an override selects a different
        // mode than the one this processor was built with, build a matching engine
        // for this request. The default path reuses the shared engine.
        let override_engine;
        let pii: &crate::pii::PiiEngine = if config.redaction.mode == self.config.redaction.mode {
            &self.pii
        } else {
            override_engine = crate::pii::PiiEngine::new(config.redaction.mode)?;
            &override_engine
        };

        match input.operation {
            EmailOperation::Archive { .. } => self.archive(input.operation, config).await,
            EmailOperation::ExtractNER { .. } => self.extract_ner(pii, input.operation, config).await,
            EmailOperation::IndexForRag { .. } => self.index_for_rag(pii, input.operation, config).await,
            EmailOperation::Redact { .. } => self.redact(pii, input.operation, config).await,
            EmailOperation::Pipeline { .. } => self.pipeline(pii, input.operation, config).await,
        }
    }

    /// Archive emails
    async fn archive(&self, op: EmailOperation, config: &EmailConfig) -> Result<EmailResponse> {
        crate::archive::archive(self.client.as_ref(), op, config).await
    }

    /// Extract named entities
    async fn extract_ner(&self, pii: &crate::pii::PiiEngine, op: EmailOperation, config: &EmailConfig) -> Result<EmailResponse> {
        crate::ner::extract_ner(self.client.as_ref(), pii, op, config).await
    }

    /// Index for RAG
    async fn index_for_rag(&self, pii: &crate::pii::PiiEngine, op: EmailOperation, config: &EmailConfig) -> Result<EmailResponse> {
        crate::rag::index_for_rag(self.client.as_ref(), pii, op, config).await
    }

    /// Redact PII
    async fn redact(&self, pii: &crate::pii::PiiEngine, op: EmailOperation, config: &EmailConfig) -> Result<EmailResponse> {
        crate::redact::redact(self.client.as_ref(), pii, op, config).await
    }

    /// Full pipeline
    async fn pipeline(&self, pii: &crate::pii::PiiEngine, op: EmailOperation, config: &EmailConfig) -> Result<EmailResponse> {
        crate::pipeline::run_pipeline(self.client.as_ref(), pii, op, config).await
    }
}