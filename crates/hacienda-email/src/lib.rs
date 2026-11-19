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
    /// Optional RAG store — the same `hacienda_rag::RagStore` document ingestion
    /// uses. When present, redacted email chunks are persisted and become
    /// searchable through the existing `/v1/rag/*` retrieve endpoints.
    rag_store: Option<Arc<dyn hacienda_rag::RagStore>>,
    /// Legal dossier used to scope ingested emails, mirroring document ingestion.
    dossier_id: Option<String>,
}

impl EmailProcessor {
    /// Create a new email processor
    pub async fn new(config: EmailConfig) -> Result<Self> {
        let client = crate::backend::build_client(&config).await?;
        let pii = crate::pii::PiiEngine::new(config.redaction.mode)?;
        Ok(Self {
            client,
            config,
            pii,
            rag_store: None,
            dossier_id: None,
        })
    }

    /// Inject the RAG store used for document ingestion so email chunks are
    /// persisted in the same searchable collections.
    pub fn with_rag_store(mut self, store: Arc<dyn hacienda_rag::RagStore>) -> Self {
        self.rag_store = Some(store);
        self
    }

    /// Scope ingested emails to a legal dossier (retrieval scoping).
    pub fn with_dossier_id(mut self, dossier_id: impl Into<String>) -> Self {
        self.dossier_id = Some(dossier_id.into());
        self
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

        // Same RAG store as document ingestion, scoped to this request's tenant
        // account and dossier. Built per call, never shared as a trusted global.
        let rag = self
            .rag_store
            .as_deref()
            .map(|store| crate::rag::RagIngestion {
                store,
                dossier_id: self.dossier_id.as_deref(),
                account: &config.account,
            });

        match input.operation {
            EmailOperation::Archive { .. } => self.archive(input.operation, config).await,
            EmailOperation::ExtractNER { .. } => {
                self.extract_ner(pii, input.operation, config).await
            }
            EmailOperation::IndexForRag { .. } => {
                self.index_for_rag(pii, input.operation, config, rag).await
            }
            EmailOperation::Redact { .. } => self.redact(pii, input.operation, config).await,
            EmailOperation::Pipeline { .. } => {
                self.pipeline(pii, input.operation, config, rag).await
            }
        }
    }

    /// Archive emails
    async fn archive(&self, op: EmailOperation, config: &EmailConfig) -> Result<EmailResponse> {
        crate::archive::archive(self.client.as_ref(), op, config).await
    }

    /// Extract named entities
    async fn extract_ner(
        &self,
        pii: &crate::pii::PiiEngine,
        op: EmailOperation,
        config: &EmailConfig,
    ) -> Result<EmailResponse> {
        crate::ner::extract_ner(self.client.as_ref(), pii, op, config).await
    }

    /// Index for RAG
    async fn index_for_rag(
        &self,
        pii: &crate::pii::PiiEngine,
        op: EmailOperation,
        config: &EmailConfig,
        rag: Option<crate::rag::RagIngestion<'_>>,
    ) -> Result<EmailResponse> {
        crate::rag::index_for_rag(self.client.as_ref(), pii, op, config, rag).await
    }

    /// Redact PII
    async fn redact(
        &self,
        pii: &crate::pii::PiiEngine,
        op: EmailOperation,
        config: &EmailConfig,
    ) -> Result<EmailResponse> {
        crate::redact::redact(self.client.as_ref(), pii, op, config).await
    }

    /// Full pipeline
    async fn pipeline(
        &self,
        pii: &crate::pii::PiiEngine,
        op: EmailOperation,
        config: &EmailConfig,
        rag: Option<crate::rag::RagIngestion<'_>>,
    ) -> Result<EmailResponse> {
        crate::pipeline::run_pipeline(self.client.as_ref(), pii, op, config, rag).await
    }
}
