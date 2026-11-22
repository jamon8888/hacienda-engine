//! hacienda-email: Email processing plugin for hacienda
//!
//! Provides email archiving, NER extraction, RAG indexing, and PII redaction
//! using the Pimalaya email ecosystem (io-email, io-imap, io-maildir, etc.).

pub mod archive;
pub mod attachments;
pub mod backend;
pub mod config;
pub mod error;
pub mod mime;
pub mod models;
pub mod native;
pub mod ner;
pub mod pii;
pub mod pipeline;
pub mod rag;
pub mod redact;
pub mod redacted;

use crate::backend::EmailClient;
use hacienda_core::auth::Caller;
use std::sync::Arc;

// Public API re-exports. A `pub use` also brings the name into this module's
// scope, so no separate private `use` is needed (doing both is E0252).
pub use crate::config::{EmailConfig, OAuthProvider, PrivacyMode};
pub use crate::error::{EmailError, Result};
pub use crate::models::{
    EmailDestination, EmailOperation, EmailRequest, EmailResponse, EmailSource, PipelineStep,
};

/// The hacienda email processor.
///
/// P0.4: the PII authority is injected by the host. The HTTP API passes the
/// `ApiState` facade (wrapped in [`crate::pii::PiiEngine`]) and the request's
/// [`Caller`] to every operation, so tenant, key and audit are never inferred
/// from a locally built facade or from `Caller::Trusted`.
pub struct EmailProcessor {
    client: Arc<dyn EmailClient>,
    config: EmailConfig,
    /// The host's tenant-scoped PII engine, shared by redaction, NER and
    /// metadata projection. Never constructed here.
    pii: crate::pii::PiiEngine,
    /// Optional RAG store — the same `hacienda_rag::RagStore` document ingestion
    /// uses. When present, redacted email chunks are persisted and become
    /// searchable through the existing `/v1/rag/*` retrieve endpoints.
    rag_store: Option<Arc<dyn hacienda_rag::RagStore>>,
    /// Legal dossier used to scope ingested emails, mirroring document ingestion.
    dossier_id: Option<String>,
}

impl EmailProcessor {
    /// Create a processor using the PII authority supplied by the host.
    ///
    /// `pii` must be built from the host facade (`PiiEngine::from_facade`);
    /// there is deliberately no constructor that builds a facade from a
    /// privacy-mode enum here.
    pub async fn new(config: EmailConfig, pii: crate::pii::PiiEngine) -> Result<Self> {
        let client = crate::backend::build_client(&config).await?;
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

    /// Process an email request as the authenticated `caller`.
    pub async fn process(&self, input: EmailRequest, caller: Caller<'_>) -> Result<EmailResponse> {
        self.process_inner(input, None, caller).await
    }

    /// Process an email request with a per-page progress callback (used by the
    /// async HTTP job to publish live progress). The callback receives
    /// `(page, pages_done, messages_done, chunks_done)` after each committed
    /// page. Other callers use [`Self::process`].
    pub async fn process_with_progress(
        &self,
        input: EmailRequest,
        on_page: &(dyn Fn(u32, usize, usize, usize) + Send + Sync),
        caller: Caller<'_>,
    ) -> Result<EmailResponse> {
        self.process_inner(input, Some(on_page), caller).await
    }

    async fn process_inner(
        &self,
        input: EmailRequest,
        on_page: Option<&(dyn Fn(u32, usize, usize, usize) + Send + Sync)>,
        caller: Caller<'_>,
    ) -> Result<EmailResponse> {
        let config = input.config_overrides.as_ref().unwrap_or(&self.config);

        // The host facade is authoritative for privacy mode, pseudonym keys and
        // audit. A request-level override can no longer rebuild a detached
        // engine (the old `PiiEngine::new` / `EnvKeyResolver` path).
        let pii = &self.pii;

        // Same RAG store as document ingestion, scoped to this request's tenant
        // account and dossier. Built per call, never shared as a trusted global.
        let rag = self
            .rag_store
            .as_deref()
            .map(|store| crate::rag::RagIngestion {
                store,
                dossier_id: self.dossier_id.as_deref(),
                account: &config.account,
                tenant_id: Some(&config.account),
                page_size: config.ingestion.page_size.max(1),
                max_pages: config.ingestion.max_pages,
                concurrency: config.ingestion.concurrency.max(1),
                incremental: config.ingestion.incremental,
                optimize_after_run: config.ingestion.optimize_after_run,
                persist_watermark: true,
                on_page,
            });

        match input.operation {
            EmailOperation::Archive { .. } => self.archive(input.operation, config).await,
            EmailOperation::ExtractNER { .. } => {
                self.extract_ner(pii, caller, input.operation, config).await
            }
            EmailOperation::IndexForRag { .. } => {
                self.index_for_rag(pii, caller, input.operation, config, rag)
                    .await
            }
            EmailOperation::Redact { .. } => {
                self.redact(pii, caller, input.operation, config).await
            }
            EmailOperation::Pipeline { .. } => {
                self.pipeline(pii, caller, input.operation, config, rag)
                    .await
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
        caller: Caller<'_>,
        op: EmailOperation,
        config: &EmailConfig,
    ) -> Result<EmailResponse> {
        crate::ner::extract_ner(self.client.as_ref(), pii, caller, op, config).await
    }

    /// Index for RAG
    async fn index_for_rag(
        &self,
        pii: &crate::pii::PiiEngine,
        caller: Caller<'_>,
        op: EmailOperation,
        config: &EmailConfig,
        rag: Option<crate::rag::RagIngestion<'_>>,
    ) -> Result<EmailResponse> {
        crate::rag::index_for_rag(self.client.as_ref(), pii, caller, op, config, rag).await
    }

    /// Redact PII
    async fn redact(
        &self,
        pii: &crate::pii::PiiEngine,
        caller: Caller<'_>,
        op: EmailOperation,
        config: &EmailConfig,
    ) -> Result<EmailResponse> {
        crate::redact::redact(self.client.as_ref(), pii, caller, op, config).await
    }

    /// Full pipeline
    async fn pipeline(
        &self,
        pii: &crate::pii::PiiEngine,
        caller: Caller<'_>,
        op: EmailOperation,
        config: &EmailConfig,
        rag: Option<crate::rag::RagIngestion<'_>>,
    ) -> Result<EmailResponse> {
        crate::pipeline::run_pipeline(self.client.as_ref(), pii, caller, op, config, rag).await
    }
}
