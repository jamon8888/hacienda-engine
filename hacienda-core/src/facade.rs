//! One call from a document to redacted text, an audit trail, and compliance artefacts.

use crate::audit::{AuditEntry, AuditEntryInput, AuditStore, InMemoryAuditStore, RedactionAction};
use crate::auth::{Caller, Capability};
use crate::compliance::{ComplianceGenerator, ComplianceReport};
use crate::config::HaciendaConfig;
use crate::error::HaciendaError;
use crate::glossary::{EntityGlossary, GlossaryEntry};
use crate::pii::{MergedEntity, PiiError, PiiPipeline, PipelineMetrics, PipelineResult};
use crate::redaction::{KeyId, KeyResolver, Pseudonymiser, RedactionError};
use crate::review::store::ReviewStore;
use crate::review::{ReviewConfig, ReviewQueue, ReviewRequest};
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};
use tokio::task::JoinSet;
use xberg::{extract, ExtractInput, ExtractionResult};

/// Version recorded on every audit entry so a record can be tied to the code that made it.
const PIPELINE_VERSION: &str = env!("CARGO_PKG_VERSION");

pub struct HaciendaFacade {
    config: HaciendaConfig,
    /// `Arc`-wrapped so [`Self::detect_concurrently`] can hand each spawned task its own
    /// cheap clone without cloning the pipeline itself (`PiiPipeline` is not `Clone`).
    pii_pipeline: Option<Arc<PiiPipeline>>,
    compliance: Option<ComplianceGenerator>,
    /// The persistence backend for the tamper-evident audit log.
    ///
    /// `None` when auditing is disabled in the config. When `Some`, every
    /// `process_batch` call makes exactly one `append` per document — the batch
    /// boundary is the whole document, not the individual entities within it.
    audit_store: Option<Arc<dyn AuditStore>>,
    review_queue: Option<ReviewQueue>,
    glossary: Option<Mutex<EntityGlossary>>,
}

/// Everything one [`HaciendaFacade::process`] call produced.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HaciendaResult {
    /// The extraction envelope. When PII is enabled, every document's `content` has
    /// already been redacted — the raw text never leaves this call.
    pub extraction: ExtractionResult,
    /// One detection result per extracted document, in the same order.
    pub pii: Vec<PipelineResult>,
    pub compliance: Option<ComplianceReport>,
    /// Audit entries appended by this call. The full chain lives in the facade.
    pub audit_entries: Vec<AuditEntry>,
    /// Detections routed to human review by this call.
    pub review_submitted: usize,
    /// Glossary terms meeting the publication threshold, across every call so far.
    pub glossary: Vec<GlossaryEntry>,
    pub metadata: HaciendaMetadata,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HaciendaMetadata {
    pub processing_time_ms: u64,
    pub pii_enabled: bool,
    pub documents: usize,
}

/// Whether detected span text should be included in a scan response.
///
/// Use `Include` only when the caller holds `Capability::PiiReveal`. Returning
/// the mention text to a caller who did not prove entitlement to it is the
/// product-killing defect that this distinction exists to prevent.
///
/// `SpanText::Include` causes an additional audit entry to be written to the
/// chain recording that raw span text was revealed, and to whom.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpanText {
    /// Clear the `text` field on every returned entity. The caller learns what
    /// *categories* of PII were found but not the exact values.
    Omit,
    /// Return the `text` field as-is. Requires `Capability::PiiReveal`.
    Include,
}

/// Result of a text-mode scan via [`HaciendaFacade::scan_text_with_auth`].
///
/// Unlike `PipelineResult`, this type never carries the raw input text and
/// guarantees that `entities[*].text` is empty when `SpanText::Omit` was
/// requested. The cleared-in-core guarantee exists so that every future caller
/// (FFI, CLI, additional transports) does not each need to re-implement
/// suppression — one of them will forget.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TextScanResult {
    /// Detected entities. `text` is cleared when `SpanText::Omit` was used.
    pub entities: Vec<MergedEntity>,
    pub metrics: PipelineMetrics,
    /// Audit entries appended by this call.
    ///
    /// For a pure scan with `SpanText::Omit`, this is empty — nothing was
    /// redacted and no reveal occurred. For `SpanText::Include`, exactly one
    /// entry is appended recording that the caller accessed span text.
    pub audit_entries: Vec<AuditEntry>,
}

/// Result of a text-mode redaction via [`HaciendaFacade::redact_text_with_auth`].
///
/// The `entities` field never carries span text — returning the plaintext of a
/// span the caller just had redacted would be self-defeating.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TextRedactResult {
    /// Input text with every merged span rewritten.
    pub redacted_text: String,
    /// Detected entities. `text` is always cleared — see struct-level doc.
    pub entities: Vec<MergedEntity>,
    pub metrics: PipelineMetrics,
    /// Audit entries appended by this call (one per redacted span).
    pub audit_entries: Vec<AuditEntry>,
}

impl HaciendaFacade {
    /// Build a facade for `config`, loading whatever the enabled stages need.
    ///
    /// Auditing uses the default [`InMemoryAuditStore`] when enabled. State is lost on
    /// drop; use [`with_stores`](Self::with_stores) and pass a [`FileAuditStore`] when
    /// the audit record must survive a restart.
    ///
    /// # Errors
    ///
    /// Returns [`HaciendaError::Pii`] if the detection pipeline cannot be built —
    /// most often because a model is enabled that this build cannot load.
    ///
    /// [`FileAuditStore`]: crate::audit::FileAuditStore
    pub fn new(config: HaciendaConfig) -> Result<Self, HaciendaError> {
        Self::build(config, None, None, None)
    }

    /// Build a facade that can mint and reverse pseudonym tokens.
    ///
    /// Required when the PII configuration selects
    /// [`RedactionMode::Pseudonymize`](crate::redaction::RedactionMode::Pseudonymize).
    /// [`HaciendaFacade::new`] passes no key and therefore *fails* for that mode; it does
    /// not fall back to masking, because that would apply a weaker control than the one
    /// the operator configured.
    ///
    /// The active key comes from
    /// [`RedactionConfig::key_id`](crate::redaction::RedactionConfig::key_id) when set and
    /// from the resolver's own notion of "active" otherwise. `retired` names keys that are
    /// no longer minted under but whose existing tokens must stay revealable — they are
    /// loaded eagerly, so a missing one is a startup error rather than a surprise during
    /// a right-of-access request.
    ///
    /// # Errors
    ///
    /// As [`HaciendaFacade::new`], plus
    /// [`HaciendaError::Pii`] wrapping a key resolution failure.
    pub fn with_key_resolver(
        config: HaciendaConfig,
        resolver: &dyn KeyResolver,
        retired: &[KeyId],
    ) -> Result<Self, HaciendaError> {
        let configured = config
            .pii
            .as_ref()
            .and_then(|p| p.redaction.key_id.as_deref())
            .map(KeyId::new)
            .transpose()
            .map_err(key_error)?;
        let pseudonymiser = match configured {
            Some(id) => Pseudonymiser::with_active(resolver, id, retired),
            None => Pseudonymiser::new(resolver, retired),
        }
        .map_err(key_error)?;
        Self::build(config, Some(Arc::new(pseudonymiser)), None, None)
    }

    /// Build a facade with explicit store backends.
    ///
    /// Use this when you need a durable audit record or a shared review store. The
    /// caller owns the store's lifetime — the facade holds an `Arc` clone and does not
    /// close the store on drop. Call [`close`](Self::close) before dropping the facade
    /// when you have supplied a `FileAuditStore`, so the open segment is sealed before
    /// the file handle is released.
    ///
    /// Supplying `None` for either store falls back to the same default as
    /// [`new`](Self::new): an in-memory audit store when auditing is enabled, and no
    /// review queue when `config.review` is `None`.
    ///
    /// Supplying a store overrides the config in both cases. A review store passed here
    /// builds a queue even when `config.review` is `None`, using
    /// [`ReviewConfig::default`] for the threshold and deadline — handing over a durable
    /// store and receiving no queue would discard every decision made against it.
    ///
    /// # Errors
    ///
    /// Returns [`HaciendaError::Pii`] if the detection pipeline cannot be built.
    pub fn with_stores(
        config: HaciendaConfig,
        audit_store: Option<Arc<dyn AuditStore>>,
        review_store: Option<Arc<dyn ReviewStore>>,
    ) -> Result<Self, HaciendaError> {
        Self::build(config, None, audit_store, review_store)
    }

    /// Shared construction. Accepts all optional overrides and applies defaults for any
    /// that are `None`.
    fn build(
        config: HaciendaConfig,
        pseudonymiser: Option<Arc<Pseudonymiser>>,
        audit_store: Option<Arc<dyn AuditStore>>,
        review_store: Option<Arc<dyn ReviewStore>>,
    ) -> Result<Self, HaciendaError> {
        let pii_pipeline = config
            .pii
            .clone()
            .map(|c| PiiPipeline::with_pseudonymiser(c, pseudonymiser.clone()))
            .transpose()?
            .map(Arc::new);

        // Auditing without detection would record nothing, so the store follows the
        // pipeline rather than being independently switchable.
        //
        // If the caller supplied a store, use it; otherwise build the default in-memory
        // store when auditing is enabled. Behaviour is unchanged for callers that use
        // `new()` or `with_key_resolver()`.
        let audit_store = if let Some(store) = audit_store {
            Some(store)
        } else {
            config
                .pii
                .as_ref()
                .filter(|p| p.audit.enabled)
                .map(|p| -> Arc<dyn AuditStore> {
                    Arc::new(InMemoryAuditStore::new(p.audit.config_hash.clone()))
                })
        };

        // Same rule as the audit arm above: an explicitly supplied store wins.
        //
        // This arm used to require `config.review` to be `Some` before it would use the
        // store at all, so a caller who passed a `FileReviewStore` under a config with no
        // review section got `review_queue() == None` — no error, no queue, and every
        // decision they went on to record vanished. A caller who hands over a durable
        // store has stated their intent unambiguously; the absent config section is the
        // weaker signal, so it yields to a default threshold rather than cancelling.
        let review_queue = match (review_store, config.review.clone()) {
            (Some(store), Some(cfg)) => Some(ReviewQueue::with_store(cfg, store)),
            (Some(store), None) => Some(ReviewQueue::with_store(ReviewConfig::default(), store)),
            (None, Some(cfg)) => Some(ReviewQueue::new(cfg)),
            (None, None) => None,
        };

        Ok(Self {
            compliance: config.compliance.clone().map(ComplianceGenerator::new),
            review_queue,
            glossary: config
                .glossary
                .clone()
                .filter(|g| g.enabled)
                .map(|g| Mutex::new(EntityGlossary::new(g))),
            pii_pipeline,
            audit_store,
            config,
        })
    }

    pub fn config(&self) -> &HaciendaConfig {
        &self.config
    }

    /// The queue holding detections that fell below the review threshold.
    pub fn review_queue(&self) -> Option<&ReviewQueue> {
        self.review_queue.as_ref()
    }

    /// Get the review queue with authentication context.
    ///
    /// Requires `review:decide` capability.
    pub fn review_queue_with_auth(
        &self,
        caller: Caller<'_>,
    ) -> Result<Option<&ReviewQueue>, HaciendaError> {
        caller.require(Capability::ReviewDecide)?;
        Ok(self.review_queue.as_ref())
    }

    /// A snapshot of the open segment's audit entries.
    ///
    /// Returns only the open segment. For the full history across sealed segments,
    /// call the store's `seals()` and re-read the segment files directly.
    ///
    /// Returns an empty `Vec` when auditing is not configured — callers do not need to
    /// distinguish "no store" from "store with no entries yet".
    pub async fn audit_entries(&self) -> Result<Vec<AuditEntry>, HaciendaError> {
        self.audit_entries_with_auth(Caller::Trusted).await
    }

    /// Get audit entries with authentication context.
    ///
    /// Requires `audit:read` capability.
    pub async fn audit_entries_with_auth(
        &self,
        caller: Caller<'_>,
    ) -> Result<Vec<AuditEntry>, HaciendaError> {
        caller.require(Capability::AuditRead)?;
        match &self.audit_store {
            Some(store) => Ok(store.entries().await?),
            None => Ok(Vec::new()),
        }
    }

    /// The current head of the audit hash chain, or `None` when auditing is disabled.
    ///
    /// Every content-bearing API response carries this so a client can prove which chain
    /// state produced a given output. Deliberately not capability-guarded: the tip is an
    /// opaque hash that reveals nothing about the entries behind it, and gating it would
    /// mean a caller with `documents:process` but not `audit:read` could not obtain the
    /// evidence for its own result.
    ///
    /// `None` is honest rather than convenient: a client must be able to tell "auditing
    /// is off, this result has no chain evidence" from "the chain is empty".
    pub async fn audit_tip(&self) -> Result<Option<String>, HaciendaError> {
        match &self.audit_store {
            Some(store) => Ok(Some(store.tip().await?)),
            None => Ok(None),
        }
    }

    /// Verify the audit chain has not been tampered with.
    ///
    /// A facade with no audit store configured trivially returns `Ok(())`. The
    /// verification is a no-op not because the chain was verified and found clean, but
    /// because there is no chain — callers that need a guarantee should assert that
    /// auditing is enabled before calling.
    ///
    /// # Errors
    ///
    /// Returns [`HaciendaError::Audit`] naming the first entry or seal whose hash does
    /// not match the chain.
    pub async fn verify_audit(&self) -> Result<(), HaciendaError> {
        self.verify_audit_with_auth(Caller::Trusted).await
    }

    /// Verify audit chain with authentication context.
    ///
    /// Requires `audit:read` capability.
    pub async fn verify_audit_with_auth(&self, caller: Caller<'_>) -> Result<(), HaciendaError> {
        caller.require(Capability::AuditRead)?;
        match &self.audit_store {
            Some(store) => Ok(store.verify().await?),
            None => Ok(()),
        }
    }

    /// Shut down every store the facade holds: seal the audit segment and release the
    /// review store's resources.
    ///
    /// Calling `close` is recommended when the facade holds a [`FileAuditStore`]: it
    /// seals the open segment so the next process finds a clean chain with no unsealed
    /// orphan. If you skip `close`, recovery on the next open replays and seals the
    /// orphan automatically — so forgetting `close` loses no data. The difference is
    /// one extra recovery step on the next startup.
    ///
    /// `close` is idempotent: calling it more than once returns `Ok(())` on every
    /// subsequent call. When no stores are configured it is a no-op.
    ///
    /// # Why both stores are closed even if the first fails
    ///
    /// A failed audit seal must not leave a review store's resources held. Both closes
    /// are attempted and the first error is returned, so one broken backend cannot make
    /// the other leak.
    ///
    /// # Why not `Drop`
    ///
    /// `Drop` cannot `.await`. `block_on` inside `Drop` panics when called from within
    /// a Tokio runtime, and a detached `tokio::spawn` risks the process exiting before
    /// the write completes. Recovery is the correct backstop; `close` is the courtesy
    /// that avoids the extra startup work.
    ///
    /// # Errors
    ///
    /// Returns [`HaciendaError::Audit`] if the seal write fails, or
    /// [`HaciendaError::Review`] if the review store's cleanup fails.
    ///
    /// [`FileAuditStore`]: crate::audit::FileAuditStore
    pub async fn close(&self) -> Result<(), HaciendaError> {
        self.close_with_auth(Caller::Trusted).await
    }

    /// Close stores with authentication context.
    ///
    /// Requires `audit:read` capability (for sealing audit chain).
    pub async fn close_with_auth(&self, caller: Caller<'_>) -> Result<(), HaciendaError> {
        caller.require(Capability::AuditRead)?;
        let audit = match &self.audit_store {
            Some(store) => store.close().await.map(|_| ()).map_err(HaciendaError::from),
            None => Ok(()),
        };

        let review = match &self.review_queue {
            Some(queue) => queue.close().await.map_err(HaciendaError::from),
            None => Ok(()),
        };

        audit.and(review)
    }

    /// Extract, detect, redact, audit, review, and generate compliance artefacts.
    ///
    /// # Errors
    ///
    /// Returns [`HaciendaError::Extraction`] if xberg cannot read the document and
    /// [`HaciendaError::Pii`] if detection fails. Detection failures are never
    /// downgraded to partial results: text that was not fully scanned must not be
    /// returned as if it had been redacted.
    pub async fn process(&self, input: ExtractInput) -> Result<HaciendaResult, HaciendaError> {
        self.process_batch(vec![input]).await
    }

    /// Process with authentication context, checking required capabilities.
    ///
    /// Requires `documents:process` capability.
    pub async fn process_with_auth(
        &self,
        caller: Caller<'_>,
        input: ExtractInput,
    ) -> Result<HaciendaResult, HaciendaError> {
        caller.require(Capability::DocumentsProcess)?;
        self.process_batch_with_auth(caller, vec![input]).await
    }

    /// Process several inputs as one extraction, sharing the audit chain and glossary.
    ///
    /// # Errors
    ///
    /// As [`HaciendaFacade::process`].
    pub async fn process_batch(
        &self,
        inputs: Vec<ExtractInput>,
    ) -> Result<HaciendaResult, HaciendaError> {
        self.process_batch_with_auth(Caller::Trusted, inputs).await
    }

    /// Process batch with authentication context, checking required capabilities.
    ///
    /// Requires `documents:process` capability.
    pub async fn process_batch_with_auth(
        &self,
        caller: Caller<'_>,
        inputs: Vec<ExtractInput>,
    ) -> Result<HaciendaResult, HaciendaError> {
        caller.require(Capability::DocumentsProcess)?;
        let start = std::time::Instant::now();

        let mut extraction = extract_all(inputs, &self.config).await?;

        let mut pii = Vec::new();
        let mut audit_entries = Vec::new();
        let mut review_submitted = 0;

        if let Some(pipeline) = &self.pii_pipeline {
            let detections = self
                .detect_concurrently(pipeline, &extraction.results)
                .await?;

            // `detect_concurrently` guarantees `detections.len() == extraction.results.len()`
            // and input order (D3), so zipping is safe and every document below is still
            // audited and reviewed on this task, one at a time, exactly as before.
            for (document, result) in extraction.results.iter_mut().zip(detections) {
                self.observe_glossary(&document.content, &result);
                audit_entries.extend(self.record_audit(&result, caller).await?);
                review_submitted += self.submit_for_review(&result).await?;

                document.content = result.redacted_text.clone();
                pii.push(result);
            }
        }

        Ok(HaciendaResult {
            compliance: self.compliance.as_ref().map(|c| c.report(None)),
            glossary: self
                .glossary
                .as_ref()
                .map(|g| lock(g).entries())
                .unwrap_or_default(),
            metadata: HaciendaMetadata {
                processing_time_ms: start.elapsed().as_millis() as u64,
                pii_enabled: self.pii_pipeline.is_some(),
                documents: extraction.results.len(),
            },
            extraction,
            pii,
            audit_entries,
            review_submitted,
        })
    }

    /// Scan raw text for PII without modifying it.
    ///
    /// Convenience delegate that passes [`Caller::Trusted`], mirroring the
    /// `process` / `process_with_auth` pairing. Use in-process tools such as
    /// the CLI where the process boundary is the trust boundary.
    ///
    /// # Errors
    ///
    /// Returns [`HaciendaError::Pii`] if PII detection is not enabled in the
    /// config (`pii` section absent) or if the detection pipeline fails.
    pub async fn scan_text(
        &self,
        text: &str,
        span_text: SpanText,
    ) -> Result<TextScanResult, HaciendaError> {
        self.scan_text_with_auth(Caller::Trusted, text, span_text)
            .await
    }

    /// Scan raw text for PII, enforcing capability requirements.
    ///
    /// # Capability requirements
    ///
    /// - `SpanText::Omit`: requires `Capability::DocumentsProcess`.
    /// - `SpanText::Include`: requires both `Capability::DocumentsProcess` and
    ///   `Capability::PiiReveal`. If `PiiReveal` is missing, an additional audit
    ///   entry recording the reveal would be impossible and the call is refused.
    ///
    /// When `SpanText::Include` is granted, an audit entry with action `Reveal`
    /// is appended to the chain. The `span_hash` on that entry is the blake3
    /// digest of the concatenated entity span texts, giving an auditable record
    /// of what was revealed and to whom without storing the plaintext.
    ///
    /// # Entity text suppression
    ///
    /// When `SpanText::Omit`, every entity's `text` field is cleared before the
    /// result leaves this method. Suppression happens here, not at the transport
    /// layer, so every future caller — FFI, CLI, a second HTTP transport — cannot
    /// accidentally omit it.
    ///
    /// # Errors
    ///
    /// Returns [`HaciendaError::Pii`] if PII is not enabled or detection fails.
    /// Returns [`HaciendaError::Authz`] if a required capability is absent.
    pub async fn scan_text_with_auth(
        &self,
        caller: Caller<'_>,
        text: &str,
        span_text: SpanText,
    ) -> Result<TextScanResult, HaciendaError> {
        caller.require(Capability::DocumentsProcess)?;
        if span_text == SpanText::Include {
            caller.require(Capability::PiiReveal)?;
        }

        let pipeline = self
            .pii_pipeline
            .as_ref()
            .ok_or(HaciendaError::PiiDisabled)?;

        let result = pipeline.scan(text).await?;

        // `PiiPipeline::scan` rewrites nothing, so its `audit_log` is empty and
        // `record_audit` would have nothing to record. The only auditable event a scan
        // can produce is the reveal itself.
        let audit_entries = match span_text {
            SpanText::Include => self.record_reveal(text, &result.entities, caller).await?,
            SpanText::Omit => Vec::new(),
        };

        let entities = if span_text == SpanText::Omit {
            result
                .entities
                .into_iter()
                .map(|mut e| {
                    e.text = String::new();
                    e
                })
                .collect()
        } else {
            result.entities
        };

        Ok(TextScanResult {
            entities,
            metrics: result.metrics,
            audit_entries,
        })
    }

    /// Redact raw text without going through document extraction.
    ///
    /// Convenience delegate that passes [`Caller::Trusted`].
    ///
    /// # Errors
    ///
    /// Returns [`HaciendaError::Pii`] if PII is not enabled or detection fails.
    pub async fn redact_text(&self, text: &str) -> Result<TextRedactResult, HaciendaError> {
        self.redact_text_with_auth(Caller::Trusted, text).await
    }

    /// Redact raw text, enforcing `Capability::DocumentsProcess`.
    ///
    /// The returned entities never carry span text — the caller has asked for the
    /// PII to be removed, so returning it in the entity list would be
    /// self-defeating. Suppression is enforced here in core rather than at the
    /// transport layer.
    ///
    /// Audit entries are written for every redacted span, matching the behaviour
    /// of `process_batch_with_auth`.
    ///
    /// # Errors
    ///
    /// Returns [`HaciendaError::Pii`] if PII is not enabled or detection fails.
    /// Returns [`HaciendaError::Authz`] if `Capability::DocumentsProcess` is absent.
    /// Returns [`HaciendaError::Audit`] if the audit write fails.
    pub async fn redact_text_with_auth(
        &self,
        caller: Caller<'_>,
        text: &str,
    ) -> Result<TextRedactResult, HaciendaError> {
        caller.require(Capability::DocumentsProcess)?;

        let pipeline = self
            .pii_pipeline
            .as_ref()
            .ok_or(HaciendaError::PiiDisabled)?;

        let result = pipeline.process(text).await?;
        let audit_entries = self.record_audit(&result, caller).await?;

        let entities = result
            .entities
            .into_iter()
            .map(|mut e| {
                e.text = String::new();
                e
            })
            .collect();

        Ok(TextRedactResult {
            redacted_text: result.redacted_text,
            entities,
            metrics: result.metrics,
            audit_entries,
        })
    }

    /// Append one `Reveal` entry per span whose plaintext was handed to `caller`.
    ///
    /// One entry per span rather than one per call, so every field is a fact about a
    /// real detection: `category`, `confidence`, and `source` describe the span, and
    /// `span_hash` is the blake3 digest of `text[start..end]` — the same digest
    /// [`crate::redaction::RedactionEngine`] records when it redacts that span. That
    /// shared digest is the point: it lets an auditor join "this value was redacted
    /// here" to "and this principal later read it", which is the question §7 exists to
    /// answer. A single per-call entry hashing the concatenation would answer neither.
    ///
    /// When no entities were found, nothing was revealed and no entry is written.
    ///
    /// # Errors
    ///
    /// [`HaciendaError::Audit`] if the store rejects the batch.
    async fn record_reveal(
        &self,
        text: &str,
        entities: &[MergedEntity],
        caller: Caller<'_>,
    ) -> Result<Vec<AuditEntry>, HaciendaError> {
        let Some(store) = &self.audit_store else {
            return Ok(Vec::new());
        };
        if entities.is_empty() {
            return Ok(Vec::new());
        }

        let principal = caller.principal_id().map(str::to_owned);
        let inputs: Vec<AuditEntryInput> = entities
            .iter()
            .map(|entity| {
                // Offsets come from the detectors that just ran over `text`, so the slice
                // is expected to be valid. Falling back to the mention text rather than
                // panicking keeps a detector bug from taking down the process; the digest
                // is still over the revealed value in either case.
                let span = text
                    .get(entity.start as usize..entity.end as usize)
                    .unwrap_or(entity.text.as_str());
                AuditEntryInput {
                    id: uuid::Uuid::new_v4().to_string(),
                    category: entity.category.to_string(),
                    action: RedactionAction::Reveal,
                    span_hash: blake3::hash(span.as_bytes()).to_hex().to_string(),
                    span_length: entity.end.saturating_sub(entity.start),
                    confidence: Some(entity.confidence),
                    source: entity.source.into(),
                    pipeline_version: PIPELINE_VERSION.to_string(),
                    // The store owns config_hash — see `record_audit`.
                    config_hash: String::new(),
                    principal: principal.clone(),
                }
            })
            .collect();

        Ok(store.append(inputs).await?)
    }

    /// Record the glossary against the *original* text, before redaction rewrites it.
    fn observe_glossary(&self, text: &str, result: &PipelineResult) {
        if let Some(glossary) = &self.glossary {
            // The guard is acquired, used synchronously, and drops before this method
            // returns. There is no `.await` in this method, so there is no risk of
            // holding a `MutexGuard` across an await point.
            lock(glossary).observe(text, &result.entities);
        }
    }

    /// Build the full batch of audit inputs for one document and append them in one call.
    ///
    /// One `append` per document is the invariant that Design Decision D3 requires. It is
    /// also what collapses §8 gap 5: there is no "first acquisition to read `config_hash`"
    /// because the store owns that field — `AuditChain::push` overwrites whatever the
    /// caller passes in with the chain's own value, so reading it before passing is a
    /// no-op that costs a lock acquisition. Setting it to `String::new()` here makes that
    /// ownership explicit.
    ///
    /// An audit entry that fails to record must not be reported back to the caller as if
    /// it succeeded — the error is propagated, not swallowed.
    ///
    /// # Errors
    ///
    /// Returns [`HaciendaError::Audit`] if the store rejects the batch.
    async fn record_audit(
        &self,
        result: &PipelineResult,
        caller: Caller<'_>,
    ) -> Result<Vec<AuditEntry>, HaciendaError> {
        let Some(store) = &self.audit_store else {
            return Ok(Vec::new());
        };

        let principal = caller.principal_id().map(str::to_owned);
        let inputs: Vec<AuditEntryInput> = result
            .audit_log
            .iter()
            .map(|entry| AuditEntryInput {
                id: uuid::Uuid::new_v4().to_string(),
                category: entry.category.clone(),
                action: entry.action.clone(),
                span_hash: entry.span_hash.clone(),
                span_length: entry.span_length,
                confidence: entry.confidence,
                source: entry.source.into(),
                pipeline_version: PIPELINE_VERSION.to_string(),
                // The store owns config_hash — AuditChain::push overwrites this field
                // with the chain's own value (chain.rs:32). Passing String::new() makes
                // the ownership explicit and removes the reason to read it first.
                config_hash: String::new(),
                principal: principal.clone(),
            })
            .collect();

        if inputs.is_empty() {
            return Ok(Vec::new());
        }

        Ok(store.append(inputs).await?)
    }

    /// Submit every low-confidence detection for human review, returning how many were
    /// accepted by the store.
    ///
    /// # Errors
    ///
    /// [`HaciendaError::Review`] if the store rejects a submission. The count is
    /// incremented only after the store confirms, and a failure aborts the document rather
    /// than being counted: `review_submitted` is reported back to the caller, and a number
    /// that counts attempts rather than acceptances would tell an operator that items are
    /// queued for review when nothing is.
    async fn submit_for_review(&self, result: &PipelineResult) -> Result<usize, HaciendaError> {
        let Some(queue) = &self.review_queue else {
            return Ok(0);
        };
        let mut count = 0;
        for entity in &result.entities {
            if queue.needs_review(entity.confidence) {
                queue
                    .submit(ReviewRequest {
                        // The snippet is the model's own mention text, which is empty for
                        // regex spans — those are deterministic and need no human context.
                        text_snippet: entity.text.clone(),
                        category: entity.category.to_string(),
                        start: entity.start,
                        end: entity.end,
                        confidence: entity.confidence,
                        source: entity.source.to_string(),
                    })
                    .await?;
                count += 1;
            }
        }
        Ok(count)
    }

    /// Run `pipeline.process` over every document, bounded by
    /// `pipeline.config().concurrency`, and return one result per document in the same
    /// order as `documents` — the contract [`HaciendaResult::pii`] documents.
    ///
    /// Only detection itself runs on a spawned task. Audit, glossary, and review
    /// recording stay on the caller's task in `process_batch_with_auth`, run once per
    /// result after this method returns them in order — so those side effects keep
    /// their original one-per-document, input-order behaviour no matter how detection
    /// completes. `self` therefore never needs to cross a spawn boundary; only
    /// `Arc<PiiPipeline>` does, which the Step 1 spike already proved is `Send + Sync`.
    ///
    /// A `concurrency` of `0` is treated as `1` — a limit that could schedule nothing
    /// would make every batch hang forever, which is a worse failure mode than the
    /// sequential behaviour a caller most likely meant.
    ///
    /// # Errors
    ///
    /// Returns the first [`HaciendaError::Pii`] raised by any document's detection.
    /// Documents already spawned when that happens are still awaited and their
    /// results discarded — nothing here can leave a straggler task detached.
    async fn detect_concurrently(
        &self,
        pipeline: &Arc<PiiPipeline>,
        documents: &[xberg::ExtractedDocument],
    ) -> Result<Vec<PipelineResult>, HaciendaError> {
        let limit = pipeline.config().concurrency.max(1);
        let texts: Vec<String> = documents.iter().map(|d| d.content.clone()).collect();
        let total = texts.len();

        let mut slots: Vec<Option<PipelineResult>> =
            std::iter::repeat_with(|| None).take(total).collect();
        let mut set: JoinSet<(usize, Result<PipelineResult, PiiError>)> = JoinSet::new();
        let mut next = 0;

        while next < total && set.len() < limit {
            spawn_detection(&mut set, pipeline, &texts, next);
            next += 1;
        }

        while let Some(joined) = set.join_next().await {
            let (index, outcome) = joined.expect("pii detection task must not panic");
            slots[index] = Some(outcome?);
            if next < total {
                spawn_detection(&mut set, pipeline, &texts, next);
                next += 1;
            }
        }

        Ok(slots
            .into_iter()
            .map(|slot| slot.expect("every index is filled before join_next returns None"))
            .collect())
    }
}

/// Spawn one detection task for `texts[index]`, tagged with `index` so the joining side
/// in [`HaciendaFacade::detect_concurrently`] can place the result regardless of which
/// task finishes first.
fn spawn_detection(
    set: &mut JoinSet<(usize, Result<PipelineResult, PiiError>)>,
    pipeline: &Arc<PiiPipeline>,
    texts: &[String],
    index: usize,
) {
    let pipeline = Arc::clone(pipeline);
    let text = texts[index].clone();
    set.spawn(async move {
        let result = pipeline.process(&text).await;
        (index, result)
    });
}

async fn extract_all(
    inputs: Vec<ExtractInput>,
    config: &HaciendaConfig,
) -> Result<ExtractionResult, HaciendaError> {
    if inputs.len() == 1 {
        let input = inputs.into_iter().next().expect("length checked above");
        return Ok(extract(input, &config.extraction).await?);
    }
    Ok(xberg::extract_batch(inputs, &config.extraction).await?)
}

/// Recover the guard when a panic poisoned the lock.
///
/// The protected state is a grow-only glossary map; it cannot be left half-written by
/// a panic in a way that corrupts future reads, so refusing to serve later requests
/// buys nothing. This helper is only used for `glossary` — the audit store has moved
/// behind `Arc<dyn AuditStore>` which handles its own interior mutability.
fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Lift a key resolution failure into the facade's error type.
///
/// `PseudonymError` sits two `#[from]` hops below `HaciendaError`, and `?` only performs
/// one, so the conversion is spelled out rather than papered over with a stringly error.
fn key_error(error: crate::redaction::PseudonymError) -> HaciendaError {
    HaciendaError::Pii(PiiError::from(RedactionError::from(error)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audit::store::AuditStore;
    use crate::audit::{AuditEntry, AuditEntryInput, AuditError, FileAuditStore, NodeId};
    use crate::glossary::GlossaryConfig;
    use crate::pii::PipelineConfig;
    use crate::redaction::{
        EnvKeyResolver, RedactionConfig, RedactionMode, ACTIVE_KEY_VAR, KEY_BYTES,
    };
    use crate::review::{
        FileReviewStore, InMemoryReviewStore, QueueStats, ReviewConfig, ReviewDecision,
        ReviewError, ReviewQueueItem, ReviewRequest, ReviewStatus,
    };
    use async_trait::async_trait;
    use std::fs;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

    // ── TempDir RAII helper ───────────────────────────────────────────────────
    // Copied from audit/store_file.rs test module. Both modules need isolation;
    // the helper is not worth making pub(crate) for six lines.

    struct TempDir(PathBuf);

    impl TempDir {
        fn new(tag: &str) -> Self {
            let path = std::env::temp_dir().join(format!(
                "hacienda-facade-{tag}-{}",
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }

        fn path(&self) -> &std::path::Path {
            &self.0
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    // ── Test double: counting store ───────────────────────────────────────────

    /// A test double that counts how many times `append` is called.
    ///
    /// The count must equal the number of documents processed — one `append` per document,
    /// never one per entity. This pins Design Decision D3's batch boundary and stops a
    /// future refactor from silently reverting to per-entity appends.
    struct CountingAuditStore {
        inner: InMemoryAuditStore,
        append_calls: AtomicUsize,
    }

    impl CountingAuditStore {
        fn new() -> Self {
            Self {
                inner: InMemoryAuditStore::new("test-config"),
                append_calls: AtomicUsize::new(0),
            }
        }

        fn append_call_count(&self) -> usize {
            self.append_calls.load(Ordering::SeqCst)
        }
    }

    #[async_trait]
    impl AuditStore for CountingAuditStore {
        async fn append(
            &self,
            inputs: Vec<AuditEntryInput>,
        ) -> Result<Vec<AuditEntry>, AuditError> {
            self.append_calls.fetch_add(1, Ordering::SeqCst);
            self.inner.append(inputs).await
        }

        async fn entries(&self) -> Result<Vec<AuditEntry>, AuditError> {
            self.inner.entries().await
        }

        async fn history(
            &self,
            after: Option<&crate::audit::AuditCursor>,
            limit: usize,
        ) -> Result<crate::audit::AuditPage, AuditError> {
            self.inner.history(after, limit).await
        }

        async fn tip(&self) -> Result<String, AuditError> {
            self.inner.tip().await
        }

        async fn seals(&self) -> Result<Vec<crate::audit::SegmentSeal>, AuditError> {
            self.inner.seals().await
        }

        async fn verify(&self) -> Result<(), AuditError> {
            self.inner.verify().await
        }

        async fn rotate(&self) -> Result<crate::audit::SegmentSeal, AuditError> {
            self.inner.rotate().await
        }

        async fn close(&self) -> Result<crate::audit::SegmentSeal, AuditError> {
            self.inner.close().await
        }
    }

    // ── Test double: review store that records its own close ──────────────────

    /// Delegates everything to an in-memory store and counts `close` calls.
    ///
    /// `ReviewStore::close` has a default no-op body, so a facade that never calls it
    /// still compiles and every other test still passes. This double is the only thing
    /// that can tell the difference.
    struct ClosingReviewStore {
        inner: InMemoryReviewStore,
        close_calls: AtomicUsize,
    }

    impl ClosingReviewStore {
        fn new() -> Self {
            Self {
                inner: InMemoryReviewStore::new(),
                close_calls: AtomicUsize::new(0),
            }
        }

        fn close_call_count(&self) -> usize {
            self.close_calls.load(Ordering::SeqCst)
        }
    }

    #[async_trait]
    impl ReviewStore for ClosingReviewStore {
        async fn submit(&self, item: ReviewQueueItem) -> Result<ReviewQueueItem, ReviewError> {
            self.inner.submit(item).await
        }

        async fn assign(&self, id: &str, reviewer: &str) -> Result<ReviewQueueItem, ReviewError> {
            self.inner.assign(id, reviewer).await
        }

        async fn decide(
            &self,
            id: &str,
            decision: ReviewDecision,
            reviewer: &str,
            comment: &str,
        ) -> Result<ReviewQueueItem, ReviewError> {
            self.inner.decide(id, decision, reviewer, comment).await
        }

        async fn list(
            &self,
            filter: Option<ReviewStatus>,
        ) -> Result<Vec<ReviewQueueItem>, ReviewError> {
            self.inner.list(filter).await
        }

        async fn get(&self, id: &str) -> Result<Option<ReviewQueueItem>, ReviewError> {
            self.inner.get(id).await
        }

        async fn stats(&self) -> Result<QueueStats, ReviewError> {
            self.inner.stats().await
        }

        async fn close(&self) -> Result<(), ReviewError> {
            self.close_calls.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }

    // ── Test double: always-failing store ─────────────────────────────────────

    /// A test double whose `append` always fails.
    ///
    /// Used to assert that an audit write failure causes `process_batch` to return `Err`
    /// rather than silently returning a result with missing entries.
    struct FailingAuditStore;

    #[async_trait]
    impl AuditStore for FailingAuditStore {
        async fn append(
            &self,
            _inputs: Vec<AuditEntryInput>,
        ) -> Result<Vec<AuditEntry>, AuditError> {
            Err(AuditError::Io {
                path: "simulated".into(),
                source: std::io::Error::other("injected failure"),
            })
        }

        async fn entries(&self) -> Result<Vec<AuditEntry>, AuditError> {
            Ok(Vec::new())
        }

        async fn history(
            &self,
            _after: Option<&crate::audit::AuditCursor>,
            _limit: usize,
        ) -> Result<crate::audit::AuditPage, AuditError> {
            Ok(crate::audit::AuditPage {
                entries: Vec::new(),
                next: None,
            })
        }

        async fn tip(&self) -> Result<String, AuditError> {
            Ok(crate::audit::GENESIS_HASH.to_owned())
        }

        async fn seals(&self) -> Result<Vec<crate::audit::SegmentSeal>, AuditError> {
            Ok(Vec::new())
        }

        async fn verify(&self) -> Result<(), AuditError> {
            Ok(())
        }

        async fn rotate(&self) -> Result<crate::audit::SegmentSeal, AuditError> {
            Err(AuditError::Io {
                path: "simulated".into(),
                source: std::io::Error::other("injected failure"),
            })
        }

        async fn close(&self) -> Result<crate::audit::SegmentSeal, AuditError> {
            Err(AuditError::Io {
                path: "simulated".into(),
                source: std::io::Error::other("injected failure"),
            })
        }
    }

    // ── Helpers ───────────────────────────────────────────────────────────────

    fn text_input(text: &str) -> ExtractInput {
        ExtractInput::from_bytes(
            text.as_bytes().to_vec(),
            "text/plain",
            Some("doc.txt".into()),
        )
    }

    fn pii_config() -> PipelineConfig {
        PipelineConfig {
            redaction: RedactionConfig {
                mode: RedactionMode::Mask,
                ..Default::default()
            },
            ..Default::default()
        }
    }

    fn pseudonymize_config(key_id: Option<&str>) -> PipelineConfig {
        PipelineConfig {
            redaction: RedactionConfig {
                mode: RedactionMode::Pseudonymize,
                key_id: key_id.map(str::to_string),
                ..Default::default()
            },
            ..Default::default()
        }
    }

    /// Holds `k1` and `k2`, reporting `k1` as active.
    fn key_resolver() -> EnvKeyResolver {
        EnvKeyResolver::with_lookup(|name| match name {
            ACTIVE_KEY_VAR => Some("k1".to_string()),
            "HACIENDA_PSEUDONYM_KEY_K1" => Some("07".repeat(KEY_BYTES)),
            "HACIENDA_PSEUDONYM_KEY_K2" => Some("a9".repeat(KEY_BYTES)),
            _ => None,
        })
    }

    fn token_in(text: &str) -> &str {
        let start = text.find('[').expect(text);
        let end = text.find(']').expect(text);
        &text[start..=end]
    }

    // ── Existing tests, updated for async audit methods ───────────────────────

    #[tokio::test]
    async fn should_redact_with_reversible_tokens_end_to_end() {
        let config = HaciendaConfig::default().with_pii(pseudonymize_config(None));
        let facade = HaciendaFacade::with_key_resolver(config, &key_resolver(), &[]).unwrap();
        let result = facade
            .process(text_input("mail bob@example.com"))
            .await
            .unwrap();

        let content = &result.extraction.results[0].content;
        assert!(!content.contains("bob@example.com"), "{content}");
        assert!(content.contains("[EMAIL:k1:"), "{content}");

        // The whole point of the mode: the value is recoverable by a key holder.
        let pseudonymiser = Pseudonymiser::new(&key_resolver(), &[]).unwrap();
        assert_eq!(
            pseudonymiser.reveal(token_in(content)).unwrap(),
            "bob@example.com"
        );
    }

    #[tokio::test]
    async fn should_give_one_value_the_same_token_across_two_documents_in_a_batch() {
        // Cross-document co-reference. If each document minted its own token, a reader
        // could not tell that the same person appears in both.
        let config = HaciendaConfig::default().with_pii(pseudonymize_config(None));
        let facade = HaciendaFacade::with_key_resolver(config, &key_resolver(), &[]).unwrap();
        let result = facade
            .process_batch(vec![
                text_input("from bob@example.com"),
                text_input("to bob@example.com"),
            ])
            .await
            .unwrap();

        let first = token_in(&result.extraction.results[0].content);
        let second = token_in(&result.extraction.results[1].content);
        assert_eq!(first, second);
    }

    #[tokio::test]
    async fn should_fail_construction_when_pseudonymize_is_configured_without_a_resolver() {
        // The fail-closed property has to survive all the way to the public entry point.
        let config = HaciendaConfig::default().with_pii(pseudonymize_config(None));
        assert!(HaciendaFacade::new(config).is_err());
    }

    #[tokio::test]
    async fn should_mint_under_the_key_named_by_configuration() {
        // Config pins the minting key rather than inheriting the resolver's active one.
        let config = HaciendaConfig::default().with_pii(pseudonymize_config(Some("k2")));
        let facade = HaciendaFacade::with_key_resolver(config, &key_resolver(), &[]).unwrap();
        let result = facade
            .process(text_input("mail bob@example.com"))
            .await
            .unwrap();
        assert!(
            result.extraction.results[0].content.contains("[EMAIL:k2:"),
            "{}",
            result.extraction.results[0].content
        );
    }

    #[tokio::test]
    async fn should_refuse_to_start_when_a_configured_key_is_missing() {
        // Better a startup failure than discovering the corpus is irreversible when a
        // data subject exercises a right of access.
        let config = HaciendaConfig::default().with_pii(pseudonymize_config(Some("gone")));
        assert!(HaciendaFacade::with_key_resolver(config, &key_resolver(), &[]).is_err());
    }

    #[tokio::test]
    async fn should_extract_without_touching_pii_when_it_is_not_configured() {
        let facade = HaciendaFacade::new(HaciendaConfig::default()).unwrap();
        let result = facade
            .process(text_input("mail bob@example.com"))
            .await
            .unwrap();

        assert!(result.pii.is_empty());
        assert!(!result.metadata.pii_enabled);
        assert!(result.extraction.results[0]
            .content
            .contains("bob@example.com"));
    }

    #[tokio::test]
    async fn should_redact_the_extracted_content_in_place() {
        let facade = HaciendaFacade::new(HaciendaConfig::default().with_pii(pii_config())).unwrap();
        let result = facade
            .process(text_input("mail bob@example.com"))
            .await
            .unwrap();

        assert!(!result.extraction.results[0]
            .content
            .contains("bob@example.com"));
        assert_eq!(result.pii.len(), 1);
        assert_eq!(result.pii[0].entities.len(), 1);
    }

    #[tokio::test]
    async fn should_append_one_audit_entry_per_redacted_span() {
        let facade = HaciendaFacade::new(HaciendaConfig::default().with_pii(pii_config())).unwrap();
        let result = facade
            .process(text_input("mail bob@example.com or amy@example.com"))
            .await
            .unwrap();

        assert_eq!(result.audit_entries.len(), 2);
        facade.verify_audit().await.unwrap();
    }

    #[tokio::test]
    async fn should_carry_the_audit_chain_across_calls() {
        let facade = HaciendaFacade::new(HaciendaConfig::default().with_pii(pii_config())).unwrap();
        facade.process(text_input("bob@example.com")).await.unwrap();
        facade.process(text_input("amy@example.com")).await.unwrap();

        assert_eq!(facade.audit_entries().await.unwrap().len(), 2);
        facade.verify_audit().await.unwrap();
    }

    #[tokio::test]
    async fn should_keep_no_audit_chain_when_auditing_is_disabled() {
        let mut config = pii_config();
        config.audit.enabled = false;
        let facade = HaciendaFacade::new(HaciendaConfig::default().with_pii(config)).unwrap();

        let result = facade.process(text_input("bob@example.com")).await.unwrap();
        assert!(result.audit_entries.is_empty());
        assert!(facade.audit_entries().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn should_not_queue_high_confidence_regex_detections_for_review() {
        let facade = HaciendaFacade::new(HaciendaConfig {
            review: Some(ReviewConfig::default()),
            ..HaciendaConfig::default().with_pii(pii_config())
        })
        .unwrap();

        let result = facade.process(text_input("bob@example.com")).await.unwrap();
        // Regex detections score 1.0, well above the default review threshold.
        assert_eq!(result.review_submitted, 0);
        assert_eq!(
            facade
                .review_queue()
                .expect("review queue is configured")
                .stats()
                .await
                .expect("stats")
                .pending,
            0
        );
    }

    #[tokio::test]
    async fn should_publish_a_term_once_it_is_seen_often_enough() {
        let facade = HaciendaFacade::new(HaciendaConfig {
            glossary: Some(GlossaryConfig::default()),
            ..HaciendaConfig::default().with_pii(pii_config())
        })
        .unwrap();

        let first = facade.process(text_input("bob@example.com")).await.unwrap();
        assert!(first.glossary.is_empty(), "one mention is below min_count");

        let second = facade.process(text_input("bob@example.com")).await.unwrap();
        assert_eq!(second.glossary.len(), 1);
        assert_eq!(second.glossary[0].term, "bob@example.com");
        assert_eq!(second.glossary[0].count, 2);
    }

    #[tokio::test]
    async fn should_generate_compliance_artefacts_when_configured() {
        let facade = HaciendaFacade::new(HaciendaConfig {
            compliance: Some(Default::default()),
            ..HaciendaConfig::default()
        })
        .unwrap();

        let report = facade
            .process(text_input("hello"))
            .await
            .unwrap()
            .compliance;
        let report = report.expect("compliance was enabled");
        assert!(report.dpia.is_some());
        assert!(report.model_card.is_some());
    }

    #[tokio::test]
    async fn should_redact_every_document_in_a_batch() {
        let facade = HaciendaFacade::new(HaciendaConfig::default().with_pii(pii_config())).unwrap();
        let result = facade
            .process_batch(vec![
                text_input("bob@example.com"),
                text_input("amy@example.com"),
            ])
            .await
            .unwrap();

        assert_eq!(result.metadata.documents, 2);
        assert_eq!(result.pii.len(), 2);
        for document in &result.extraction.results {
            assert!(!document.content.contains('@'));
        }
    }

    // ── New tests for Task 7 ──────────────────────────────────────────────────

    /// Exactly one `append` call per document, not one per entity.
    ///
    /// This pins Design Decision D3. If a future refactor reverts to calling `append`
    /// per entity, this test fails immediately — before any profiling or audit-log
    /// analysis is needed to notice the regression. Two addresses in one document
    /// produce two audit entries but still only one `append` call.
    #[tokio::test]
    async fn should_append_exactly_once_per_document() {
        let counting_store = Arc::new(CountingAuditStore::new());
        let facade = HaciendaFacade::with_stores(
            HaciendaConfig::default().with_pii(pii_config()),
            Some(Arc::clone(&counting_store) as Arc<dyn AuditStore>),
            None,
        )
        .unwrap();

        // One document with two PII spans → still one append call.
        facade
            .process(text_input("bob@example.com and amy@example.com"))
            .await
            .unwrap();

        assert_eq!(
            counting_store.append_call_count(),
            1,
            "one document must produce exactly one append call"
        );

        // A second document → second append call, total two.
        facade
            .process(text_input("carol@example.com"))
            .await
            .unwrap();

        assert_eq!(
            counting_store.append_call_count(),
            2,
            "two documents must produce exactly two append calls"
        );
    }

    /// The audit chain is preserved across a facade restart when backed by a FileAuditStore.
    ///
    /// Build a facade, process a document so entries are written to disk, call `close` to
    /// seal the segment cleanly, drop the facade, then rebuild a new facade pointing at the
    /// same root directory and assert that the earlier entries are still verifiable.
    #[tokio::test]
    async fn should_keep_the_audit_chain_across_a_facade_restart() {
        let dir = TempDir::new("restart");
        let node = NodeId::new("test-node");
        // Read the hash off the config rather than hardcoding it. The store and the
        // facade must be opened under the *same* config hash or recovery fails with
        // `ConfigMismatch` — and a hardcoded literal would turn a future change to
        // `AuditConfig::default()` into a confusing mismatch error here instead of a
        // plain assertion failure.
        let config = HaciendaConfig::default().with_pii(pii_config());
        let config_hash = config
            .pii
            .as_ref()
            .expect("pii_config sets a pii section")
            .audit
            .config_hash
            .clone();

        let entry_count_before;

        // First run: process a document, close cleanly.
        {
            let store = Arc::new(
                FileAuditStore::open(dir.path(), node.clone(), &config_hash)
                    .expect("open store for first run"),
            );
            let facade = HaciendaFacade::with_stores(
                config.clone(),
                Some(Arc::clone(&store) as Arc<dyn AuditStore>),
                None,
            )
            .unwrap();

            facade
                .process(text_input("bob@example.com"))
                .await
                .expect("process first run");

            entry_count_before = facade
                .audit_entries()
                .await
                .expect("entries first run")
                .len();
            assert!(entry_count_before > 0, "must have at least one entry");

            facade.close().await.expect("close first run");
        }
        // Store and facade both dropped here.

        // Second run: open a new store at the same root, recover, and verify.
        {
            let store = Arc::new(
                FileAuditStore::open(dir.path(), node, &config_hash)
                    .expect("open store for second run — recovery must succeed"),
            );
            let facade = HaciendaFacade::with_stores(
                config.clone(),
                Some(Arc::clone(&store) as Arc<dyn AuditStore>),
                None,
            )
            .unwrap();

            // The chain from the first run is in the sealed segment; verify must pass.
            facade
                .verify_audit()
                .await
                .expect("chain from first run must still verify after restart");

            // The sealed segment carries the entry count from before the restart.
            let seals = store.seals().await.expect("seals");
            let total_sealed: u64 = seals.iter().map(|s| s.entry_count).sum();
            assert_eq!(
                total_sealed, entry_count_before as u64,
                "sealed entry count must equal what was written in the first run"
            );
        }
    }

    /// Review decisions survive a facade restart when backed by a `FileReviewStore`.
    ///
    /// The audit chain has had this coverage since Phase 1; the review queue never did,
    /// although the Phase 1 plan claimed otherwise. Without it, nothing pinned the review
    /// half of `with_stores` — which is exactly where the store was being dropped (#25).
    ///
    /// Modelled on `should_keep_the_audit_chain_across_a_facade_restart`: submit and
    /// decide in the first run, drop everything, reopen on the same file, and assert the
    /// decision is still there.
    #[tokio::test]
    async fn should_keep_review_decisions_across_a_facade_restart() {
        let dir = TempDir::new("review-restart");
        let log = dir.path().join("review.jsonl");
        let config = HaciendaConfig::default().with_pii(pii_config());

        let item_id;

        // First run: submit an item and decide it.
        {
            let store = Arc::new(FileReviewStore::open(&log).expect("open store for first run"));
            let facade = HaciendaFacade::with_stores(
                config.clone(),
                None,
                Some(Arc::clone(&store) as Arc<dyn ReviewStore>),
            )
            .unwrap();

            let queue = facade
                .review_queue()
                .expect("an explicitly supplied review store must produce a queue");

            let item = queue
                .submit(ReviewRequest {
                    text_snippet: "bob@example.com".into(),
                    category: "Email".into(),
                    start: 0,
                    end: 15,
                    confidence: 0.4,
                    source: "regex".into(),
                })
                .await
                .expect("submit first run");
            item_id = item.id.clone();

            queue
                .decide(&item_id, ReviewDecision::Approve, "amy", "looks right")
                .await
                .expect("decide first run");

            facade.close().await.expect("close first run");
        }
        // Store and facade both dropped here.

        // Second run: reopen the same log and read the decision back.
        {
            let store = Arc::new(
                FileReviewStore::open(&log).expect("open store for second run — replay must work"),
            );
            let facade = HaciendaFacade::with_stores(
                config,
                None,
                Some(Arc::clone(&store) as Arc<dyn ReviewStore>),
            )
            .unwrap();

            let item = facade
                .review_queue()
                .expect("queue after restart")
                .get(&item_id)
                .await
                .expect("get after restart")
                .expect("the item written in the first run must still exist");

            assert_eq!(item.decision, Some(ReviewDecision::Approve));
            assert_eq!(item.decided_by.as_deref(), Some("amy"));
            assert_eq!(item.status, ReviewStatus::Approved);
        }
    }

    /// `HaciendaFacade::close` closes the review store, not only the audit store.
    ///
    /// `close` covered the audit store alone. Because `ReviewStore::close` has a default
    /// no-op body, nothing else in the suite can distinguish "closed" from "skipped" —
    /// this double counts the calls. The backends that exist today hold no resource, so
    /// the omission is currently harmless; a Postgres pool in Phase 6 would leak.
    #[tokio::test]
    async fn should_close_the_review_store_along_with_the_audit_store() {
        let store = Arc::new(ClosingReviewStore::new());
        let facade = HaciendaFacade::with_stores(
            HaciendaConfig::default().with_pii(pii_config()),
            None,
            Some(Arc::clone(&store) as Arc<dyn ReviewStore>),
        )
        .unwrap();

        facade.close().await.expect("close");
        assert_eq!(store.close_call_count(), 1, "close must reach the store");
    }

    /// An explicitly supplied review store is used even when `config.review` is `None`.
    ///
    /// `with_stores` used to drop the store on this path, leaving `review_queue()` as
    /// `None` — a caller who passed a `FileReviewStore` got silence, not an error, and
    /// every subsequent decision went nowhere. The audit arm immediately above resolves
    /// the same conflict the other way: an explicit argument wins. This test pins the
    /// two arms to the same rule.
    #[tokio::test]
    async fn should_use_an_explicit_review_store_when_the_config_has_no_review_section() {
        let config = HaciendaConfig::default().with_pii(pii_config());
        assert!(config.review.is_none(), "precondition for this test");

        let store = Arc::new(InMemoryReviewStore::new());
        let facade =
            HaciendaFacade::with_stores(config, None, Some(store as Arc<dyn ReviewStore>)).unwrap();

        let queue = facade.review_queue().expect(
            "passing a review store must build a queue; dropping it silently loses every decision",
        );
        queue
            .submit(ReviewRequest {
                text_snippet: "bob@example.com".into(),
                category: "Email".into(),
                start: 0,
                end: 15,
                confidence: 0.4,
                source: "regex".into(),
            })
            .await
            .expect("submit must reach the supplied store");

        assert_eq!(queue.stats().await.expect("stats").total, 1);
    }

    /// `verify_audit` on a facade with no audit store returns `Ok(())`.
    ///
    /// This tests the explicit `None` arm, not just the happy path. A facade whose
    /// `verify_audit` panics or returns an error when auditing is disabled is wrong.
    #[tokio::test]
    async fn should_return_ok_from_verify_audit_when_no_store_is_configured() {
        let facade = HaciendaFacade::new(HaciendaConfig::default()).unwrap();
        // No PII config means no audit store.
        facade
            .verify_audit()
            .await
            .expect("verify_audit with no store must return Ok(())");
    }

    /// `close` is idempotent: a second call returns `Ok(())` without error.
    ///
    /// Shutdown sequences often call `close` more than once. Both the in-memory and file
    /// backends document `AuditStore::close` as idempotent; this test asserts that the
    /// facade passes the idempotence through correctly.
    #[tokio::test]
    async fn should_be_idempotent_on_close() {
        let facade = HaciendaFacade::new(HaciendaConfig::default().with_pii(pii_config())).unwrap();
        facade.process(text_input("bob@example.com")).await.unwrap();

        facade.close().await.expect("first close");
        facade
            .close()
            .await
            .expect("second close must also succeed");
    }

    /// An audit store failure during `process_batch` propagates as an error.
    ///
    /// A caller must not receive a `HaciendaResult` whose `audit_entries` is empty
    /// because the write silently failed — that would let the facade report "0 entries
    /// audited" when persistence is actually broken. The `FailingAuditStore` injects an
    /// `Io` error on every `append`, and we assert the whole batch returns `Err`.
    #[tokio::test]
    async fn should_fail_the_batch_when_the_audit_store_fails() {
        let failing_store = Arc::new(FailingAuditStore);
        let facade = HaciendaFacade::with_stores(
            HaciendaConfig::default().with_pii(pii_config()),
            Some(failing_store as Arc<dyn AuditStore>),
            None,
        )
        .unwrap();

        let result = facade.process(text_input("bob@example.com")).await;
        assert!(
            result.is_err(),
            "a failing audit store must cause process to return Err"
        );
        match result.unwrap_err() {
            HaciendaError::Audit(_) => {}
            other => panic!("expected HaciendaError::Audit, got {other:?}"),
        }
    }

    /// `close` on a facade with no audit store is a no-op returning `Ok(())`.
    #[tokio::test]
    async fn should_return_ok_from_close_when_no_store_is_configured() {
        let facade = HaciendaFacade::new(HaciendaConfig::default()).unwrap();
        facade
            .close()
            .await
            .expect("close with no store must return Ok(())");
    }

    // ── Tests for scan_text / redact_text (Task 1) ───────────────────────────

    use crate::auth::AuthContext;
    use crate::pii::NerDetector;
    use xberg::text::ner::NerBackend;
    use xberg::types::entity::{Entity, EntityCategory};
    use xberg::Result as XbergResult;

    /// A detector whose response is fixed at construction time. Used to inject
    /// model entities (which carry non-empty `text`) without loading a real model.
    struct FixedDetector(Vec<Entity>);

    #[cfg_attr(not(target_arch = "wasm32"), async_trait::async_trait)]
    #[cfg_attr(target_arch = "wasm32", async_trait::async_trait(?Send))]
    impl NerBackend for FixedDetector {
        async fn detect(
            &self,
            _text: &str,
            _categories: &[EntityCategory],
        ) -> XbergResult<Vec<Entity>> {
            Ok(self.0.clone())
        }
    }

    /// Build a facade that injects a single model entity so the `text` field is
    /// non-empty. Regex detections have `text == ""`, so a test that only uses
    /// regex spans would pass even if the clearing logic were absent.
    fn facade_with_model_entity(entity_text: &str) -> HaciendaFacade {
        let entity = Entity {
            category: EntityCategory::Person,
            text: entity_text.into(),
            start: 0,
            end: entity_text.len() as u32,
            confidence: Some(0.95),
        };
        let detector = NerDetector::new(Arc::new(FixedDetector(vec![entity])));
        let pipeline = crate::pii::PiiPipeline::with_detector(pii_config(), Some(detector))
            .expect("pipeline builds");
        // No public constructor injects a pre-built pipeline — callers are not meant to
        // own one. The struct is crate-visible, so the test assembles it directly rather
        // than widening the public API for a test's benefit.
        let config = HaciendaConfig::default().with_pii(pii_config());
        let audit_store: Option<Arc<dyn AuditStore>> = Some(Arc::new(InMemoryAuditStore::new(
            config
                .pii
                .as_ref()
                .map(|p| p.audit.config_hash.as_str())
                .unwrap_or("default"),
        )));
        HaciendaFacade {
            config,
            pii_pipeline: Some(Arc::new(pipeline)),
            compliance: None,
            audit_store,
            review_queue: None,
            glossary: None,
        }
    }

    fn principal_with(caps: &[Capability]) -> AuthContext {
        AuthContext::new(
            "test-principal",
            crate::auth::CapabilitySet::new(caps.iter().copied()),
        )
    }

    /// A principal with `DocumentsProcess` but not `PiiReveal` is refused when
    /// `SpanText::Include` is requested.
    #[tokio::test]
    async fn scan_text_include_requires_pii_reveal_capability() {
        let facade = HaciendaFacade::new(HaciendaConfig::default().with_pii(pii_config())).unwrap();
        let ctx = principal_with(&[Capability::DocumentsProcess]);
        let caller = Caller::Principal(&ctx);

        let result = facade
            .scan_text_with_auth(caller, "bob@example.com", SpanText::Include)
            .await;

        assert!(result.is_err(), "must be rejected when PiiReveal is absent");
        assert!(
            matches!(result.unwrap_err(), HaciendaError::Authz(_)),
            "error must be Authz, not a different kind"
        );
    }

    /// The same principal is accepted with `SpanText::Omit` (no `PiiReveal` needed).
    #[tokio::test]
    async fn scan_text_omit_requires_only_documents_process() {
        let facade = HaciendaFacade::new(HaciendaConfig::default().with_pii(pii_config())).unwrap();
        let ctx = principal_with(&[Capability::DocumentsProcess]);
        let caller = Caller::Principal(&ctx);

        let result = facade
            .scan_text_with_auth(caller, "bob@example.com", SpanText::Omit)
            .await;

        assert!(
            result.is_ok(),
            "must succeed with DocumentsProcess and SpanText::Omit"
        );
    }

    /// `SpanText::Omit` clears the `text` field on model entities.
    ///
    /// Regex detections already have `text == ""` (documented on `MergedEntity`),
    /// so this test uses a fixed-detector facade that injects a model entity with
    /// a known name. If clearing were absent the name would leak; with clearing it
    /// must be gone.
    ///
    /// The test also verifies the detection itself was non-empty (start/end > 0 or
    /// category set) so the assertion does not pass vacuously because no entity was
    /// found.
    #[tokio::test]
    async fn scan_text_omit_clears_entity_text_from_model_detections() {
        // "Alice Martin" is the entity text the fixed detector will return.
        let facade = facade_with_model_entity("Alice Martin");
        // Input must contain the span for offsets to be valid.
        let text = "Alice Martin signed the lease";

        let result = facade
            .scan_text(text, SpanText::Omit)
            .await
            .expect("scan should succeed");

        assert!(
            !result.entities.is_empty(),
            "detector must find Alice Martin"
        );
        for entity in &result.entities {
            assert!(
                entity.text.is_empty(),
                "SpanText::Omit must clear entity.text; got {:?}",
                entity.text
            );
        }
    }

    /// `SpanText::Include` returns entity text (non-empty for model detections).
    #[tokio::test]
    async fn scan_text_include_returns_entity_text() {
        let facade = facade_with_model_entity("Alice Martin");
        let text = "Alice Martin signed the lease";

        // Trusted caller bypasses capability checks.
        let result = facade
            .scan_text(text, SpanText::Include)
            .await
            .expect("scan should succeed");

        assert!(
            !result.entities.is_empty(),
            "detector must find Alice Martin"
        );
        let model_entity = result
            .entities
            .iter()
            .find(|e| !e.text.is_empty())
            .expect("at least one model entity must carry text with SpanText::Include");
        assert_eq!(model_entity.text, "Alice Martin");
    }

    /// Scanning must not alter the input string.
    ///
    /// `PipelineResult::redacted_text` is documented as "equal to the input for
    /// scan", but this test asserts the contract at the facade level where a future
    /// refactor could accidentally overwrite it.
    #[tokio::test]
    async fn scan_text_does_not_mutate_input() {
        let facade = HaciendaFacade::new(HaciendaConfig::default().with_pii(pii_config())).unwrap();
        let text = "contact bob@example.com for details";
        // Make an owned copy so we can compare the original bytes later.
        let original = text.to_string();

        facade
            .scan_text(text, SpanText::Omit)
            .await
            .expect("scan should succeed");

        assert_eq!(text, original.as_str(), "scan must not alter the input");
    }

    /// `redact_text_with_auth` returns the rewritten string and clears entity text.
    #[tokio::test]
    async fn redact_text_returns_redacted_string_and_clears_entity_text() {
        let facade = HaciendaFacade::new(HaciendaConfig::default().with_pii(pii_config())).unwrap();
        let result = facade
            .redact_text("contact bob@example.com for details")
            .await
            .expect("redact should succeed");

        assert!(
            !result.redacted_text.contains("bob@example.com"),
            "email must be redacted: {}",
            result.redacted_text
        );
        for entity in &result.entities {
            assert!(
                entity.text.is_empty(),
                "redact_text must never return entity text; got {:?}",
                entity.text
            );
        }
        assert!(
            !result.audit_entries.is_empty(),
            "redaction must be audited"
        );
    }

    /// A facade with PII disabled returns an error, not an empty success.
    ///
    /// Returning "no PII found" when the detector was never enabled would be the
    /// worst possible failure mode for a redaction product. The test covers both
    /// `scan_text` and `redact_text`.
    #[tokio::test]
    async fn scan_and_redact_text_return_error_when_pii_is_not_configured() {
        // No `.with_pii(...)` call — PII pipeline is `None`.
        let facade = HaciendaFacade::new(HaciendaConfig::default()).unwrap();

        let scan_result = facade.scan_text("bob@example.com", SpanText::Omit).await;
        assert!(
            scan_result.is_err(),
            "scan_text must error when PII is not configured"
        );
        assert!(
            matches!(scan_result.unwrap_err(), HaciendaError::PiiDisabled),
            "error must name the misconfiguration, not a detection failure"
        );

        let redact_result = facade.redact_text("bob@example.com").await;
        assert!(
            redact_result.is_err(),
            "redact_text must error when PII is not configured"
        );
        assert!(
            matches!(redact_result.unwrap_err(), HaciendaError::PiiDisabled),
            "error must name the misconfiguration, not a detection failure"
        );
    }

    /// `SpanText::Include` writes an audit entry for the reveal event.
    #[tokio::test]
    async fn scan_text_include_writes_reveal_audit_entry() {
        let facade = facade_with_model_entity("Alice Martin");
        let text = "Alice Martin signed the lease";

        let result = facade
            .scan_text(text, SpanText::Include)
            .await
            .expect("scan should succeed");

        assert!(
            !result.audit_entries.is_empty(),
            "a reveal event must produce an audit entry"
        );
        let reveal = result
            .audit_entries
            .iter()
            .find(|e| e.action == crate::audit::RedactionAction::Reveal)
            .expect("must have a Reveal action entry");
        // The digest must be over the revealed span, not over a placeholder: an entry
        // whose span_hash is the digest of "" would record that *something* was revealed
        // while being unable to say what.
        assert_eq!(
            reveal.span_hash,
            blake3::hash("Alice Martin".as_bytes()).to_hex().to_string(),
            "reveal entry must hash the span that was actually revealed"
        );
        assert_eq!(reveal.category, "Person");
    }

    /// The `PiiReveal` audit entry names the principal who accessed the span text.
    #[tokio::test]
    async fn scan_text_include_reveal_entry_names_principal() {
        let facade = facade_with_model_entity("Alice Martin");
        let text = "Alice Martin signed the lease";
        let ctx = principal_with(&[Capability::DocumentsProcess, Capability::PiiReveal]);
        let caller = Caller::Principal(&ctx);

        let result = facade
            .scan_text_with_auth(caller, text, SpanText::Include)
            .await
            .expect("scan should succeed");

        let reveal = result
            .audit_entries
            .iter()
            .find(|e| e.action == crate::audit::RedactionAction::Reveal)
            .expect("reveal entry must exist");
        assert_eq!(
            reveal.principal.as_deref(),
            Some("test-principal"),
            "reveal entry must name the accessing principal"
        );
    }

    /// `SpanText::Omit` produces no reveal audit entries (nothing was revealed).
    #[tokio::test]
    async fn scan_text_omit_writes_no_audit_entries() {
        let facade = facade_with_model_entity("Alice Martin");
        let text = "Alice Martin signed the lease";

        let result = facade
            .scan_text(text, SpanText::Omit)
            .await
            .expect("scan should succeed");

        assert!(
            result.audit_entries.is_empty(),
            "SpanText::Omit must produce no audit entries; got {}",
            result.audit_entries.len()
        );
    }

    // ── Concurrency spike (Task 4, Step 1 — closes #30) ─────────────────────────
    //
    // Throwaway proof-of-compile, not part of `process_batch_with_auth`. Every
    // `.process()` call and every `record_audit` / `observe_glossary` /
    // `submit_for_review` call below runs on an independently spawned task, holding
    // only `Arc<PiiPipeline>` and `Arc<HaciendaFacade>`. If this stops compiling,
    // some field inside `PiiPipeline` (most likely `NerDetector`'s boxed backend) or
    // inside `HaciendaFacade` (the audit store, glossary mutex, or review queue) is
    // not `Send + Sync`, and the pool shape planned for Step 4 must change.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn spike_pii_pipeline_and_facade_survive_concurrent_spawns() {
        use tokio::task::JoinSet;

        const DOCS: usize = 8;
        let texts = [
            "write to bob@example.com today",
            "contact alice@example.com now",
            "reach carol@example.com please",
            "email dave@example.com asap",
        ];

        let pipeline = Arc::new(PiiPipeline::with_detector(pii_config(), None).unwrap());

        let mut config = HaciendaConfig::default().with_pii(pii_config());
        // Force every regex detection (confidence 1.0) into the review queue, so the
        // spike exercises `submit_for_review` as well as `record_audit`.
        config.review = Some(ReviewConfig {
            confidence_threshold: 1.5,
            deadline_hours: Some(1),
        });
        config.glossary = Some(GlossaryConfig::default());
        let facade = Arc::new(HaciendaFacade::new(config).unwrap());

        let mut set: JoinSet<Result<usize, HaciendaError>> = JoinSet::new();
        for i in 0..DOCS {
            let pipeline = Arc::clone(&pipeline);
            let facade = Arc::clone(&facade);
            let text = texts[i % texts.len()].to_string();
            set.spawn(async move {
                let result = pipeline.process(&text).await?;
                facade.observe_glossary(&text, &result);
                let audit_entries = facade.record_audit(&result, Caller::Trusted).await?;
                let submitted = facade.submit_for_review(&result).await?;
                Ok(audit_entries.len() + submitted)
            });
        }

        let mut activity = 0;
        while let Some(res) = set.join_next().await {
            activity += res
                .expect("spawned task must not panic")
                .expect("stage must not error");
        }

        assert_eq!(
            activity,
            DOCS * 2,
            "each of the {DOCS} documents should record one audit entry and one review submission"
        );
    }

    // ── Task 4, Step 2 — pii results stay in input order under concurrency > 1 ──
    //
    // A detector that sleeps for a caller-supplied duration before returning, keyed by
    // which document's text it was asked to detect on. Document 0 sleeps longest and
    // document 3 does not sleep at all, so under real concurrency document 0 is the
    // *last* task to finish even though it must be the *first* entry in
    // `HaciendaResult::pii` (the contract documented on that field). A collector that
    // pushes results as they finish — the naive, easy-to-write alternative to indexed
    // collection — would return document 3's content first and document 0's last; only
    // collecting by index returns them in input order regardless of completion order.
    struct DelayedDetector {
        delays: Vec<(&'static str, std::time::Duration)>,
    }

    #[cfg_attr(not(target_arch = "wasm32"), async_trait::async_trait)]
    #[cfg_attr(target_arch = "wasm32", async_trait::async_trait(?Send))]
    impl NerBackend for DelayedDetector {
        async fn detect(
            &self,
            text: &str,
            _categories: &[EntityCategory],
        ) -> XbergResult<Vec<Entity>> {
            if let Some((_, delay)) = self.delays.iter().find(|(tag, _)| text.contains(tag)) {
                tokio::time::sleep(*delay).await;
            }
            Ok(Vec::new())
        }
    }

    /// Build a facade around a hand-assembled pipeline, bypassing `HaciendaFacade::new`
    /// the same way `facade_with_model_entity` does above — there is no public
    /// constructor that accepts both a caller-supplied detector and a non-default
    /// `concurrency`, and there does not need to be one just for this test.
    fn facade_with_pipeline(
        pipeline: PiiPipeline,
        audit_store: Option<Arc<dyn AuditStore>>,
    ) -> HaciendaFacade {
        let config = HaciendaConfig::default().with_pii(pipeline.config().clone());
        HaciendaFacade {
            config,
            pii_pipeline: Some(Arc::new(pipeline)),
            compliance: None,
            audit_store,
            review_queue: None,
            glossary: None,
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn should_return_pii_results_in_input_order_under_concurrency() {
        const TAGS: [&str; 4] = ["doc-0", "doc-1", "doc-2", "doc-3"];
        let texts: Vec<String> = TAGS
            .iter()
            .map(|tag| format!("{tag} contains no personal data at all"))
            .collect();

        let detector = NerDetector::new(Arc::new(DelayedDetector {
            delays: vec![
                ("doc-0", std::time::Duration::from_millis(300)),
                ("doc-1", std::time::Duration::from_millis(200)),
                ("doc-2", std::time::Duration::from_millis(100)),
                ("doc-3", std::time::Duration::from_millis(0)),
            ],
        }));

        let mut config = pii_config();
        config.concurrency = 4; // >= document count, so every task is in flight at once
        let pipeline = PiiPipeline::with_detector(config, Some(detector)).unwrap();
        let facade = facade_with_pipeline(pipeline, None);

        let inputs = texts.iter().map(|t| text_input(t)).collect();
        let result = facade.process_batch(inputs).await.unwrap();

        let redacted: Vec<&str> = result
            .pii
            .iter()
            .map(|r| r.redacted_text.as_str())
            .collect();
        let expected: Vec<&str> = texts.iter().map(String::as_str).collect();
        assert_eq!(
            redacted, expected,
            "pii results must stay in input order even though doc-0 (index 0) is the \
             slowest task to finish and doc-3 (index 3) is the fastest"
        );
    }

    // ── Task 4, Step 3 — every document is audited exactly once under concurrency > 1 ──
    //
    // Reuses `CountingAuditStore` (defined above for the sequential version of this same
    // invariant). A worker pool that silently drops a document — e.g. on a full bounded
    // channel — is a compliance defect, not a performance one: this asserts an exact
    // count equal to the number of documents, not merely "at least one" or "no panic".
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn should_audit_every_document_exactly_once_under_concurrency() {
        const DOCS: usize = 8;
        let counting_store = Arc::new(CountingAuditStore::new());

        let mut config = pii_config();
        config.concurrency = 4;
        let facade = HaciendaFacade::with_stores(
            HaciendaConfig::default().with_pii(config),
            Some(Arc::clone(&counting_store) as Arc<dyn AuditStore>),
            None,
        )
        .unwrap();

        let inputs = (0..DOCS)
            .map(|i| text_input(&format!("user{i}@example.com")))
            .collect();
        facade.process_batch(inputs).await.unwrap();

        assert_eq!(
            counting_store.append_call_count(),
            DOCS,
            "every document must be audited exactly once, even under concurrency"
        );
    }

    // ── Task 4, Step 6 (strengthening) — the concurrency limit is actually enforced ──
    //
    // Step 6's mutation exercise (setting `limit = usize::MAX` by hand) left every
    // existing test green, which per the plan means "the bound is untested and Step 3
    // needs strengthening" — `should_audit_every_document_exactly_once_under_concurrency`
    // proves the pool doesn't *drop* documents, but a pool that ignores its configured
    // limit and runs everything at once would pass it too. This test tracks how many
    // detections are simultaneously in flight and asserts the observed peak equals the
    // configured limit exactly: not "at most", which an accidentally-serial pool would
    // also satisfy, and not "at least", which an unbounded pool would also satisfy.
    struct TrackingDetector {
        in_flight: Arc<AtomicUsize>,
        peak: Arc<AtomicUsize>,
        delay: std::time::Duration,
    }

    #[cfg_attr(not(target_arch = "wasm32"), async_trait::async_trait)]
    #[cfg_attr(target_arch = "wasm32", async_trait::async_trait(?Send))]
    impl NerBackend for TrackingDetector {
        async fn detect(
            &self,
            _text: &str,
            _categories: &[EntityCategory],
        ) -> XbergResult<Vec<Entity>> {
            let now = self.in_flight.fetch_add(1, Ordering::SeqCst) + 1;
            self.peak.fetch_max(now, Ordering::SeqCst);
            tokio::time::sleep(self.delay).await;
            self.in_flight.fetch_sub(1, Ordering::SeqCst);
            Ok(Vec::new())
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 8)]
    async fn should_bound_in_flight_detections_to_the_configured_concurrency_limit() {
        const DOCS: usize = 8;
        const LIMIT: usize = 2;

        let in_flight = Arc::new(AtomicUsize::new(0));
        let peak = Arc::new(AtomicUsize::new(0));
        let detector = NerDetector::new(Arc::new(TrackingDetector {
            in_flight: Arc::clone(&in_flight),
            peak: Arc::clone(&peak),
            delay: std::time::Duration::from_millis(50),
        }));

        let mut config = pii_config();
        config.concurrency = LIMIT;
        let pipeline = PiiPipeline::with_detector(config, Some(detector)).unwrap();
        let facade = facade_with_pipeline(pipeline, None);

        let inputs = (0..DOCS)
            .map(|i| text_input(&format!("doc {i} has no personal data")))
            .collect();
        facade.process_batch(inputs).await.unwrap();

        assert_eq!(
            peak.load(Ordering::SeqCst),
            LIMIT,
            "peak in-flight detections must equal the configured concurrency limit \
             ({LIMIT}) — not fewer (the pool would be under-using its budget) and not \
             more (the pool would be ignoring it), across {DOCS} documents"
        );
    }

    // ── Task 5: the §9 measurement that gates Phase 6 ────────────────────────
    //
    // Wraps a `FileAuditStore` to time each `append` call end-to-end (wait + mint +
    // write + fsync) — D2's number (2). `FileAuditStore::io_order_wait` already
    // reports number (3) from inside the lock; this decorator adds number (2) at the
    // call boundary rather than adding another always-on field to the store itself.
    struct TimingAuditStore {
        inner: Arc<FileAuditStore>,
        append_nanos: AtomicU64,
    }

    impl TimingAuditStore {
        fn new(inner: Arc<FileAuditStore>) -> Self {
            Self {
                inner,
                append_nanos: AtomicU64::new(0),
            }
        }

        fn total_append_time(&self) -> std::time::Duration {
            std::time::Duration::from_nanos(self.append_nanos.load(Ordering::SeqCst))
        }
    }

    #[async_trait]
    impl AuditStore for TimingAuditStore {
        async fn append(
            &self,
            inputs: Vec<AuditEntryInput>,
        ) -> Result<Vec<AuditEntry>, AuditError> {
            let start = std::time::Instant::now();
            let result = self.inner.append(inputs).await;
            self.append_nanos
                .fetch_add(start.elapsed().as_nanos() as u64, Ordering::SeqCst);
            result
        }

        async fn entries(&self) -> Result<Vec<AuditEntry>, AuditError> {
            self.inner.entries().await
        }

        async fn history(
            &self,
            after: Option<&crate::audit::AuditCursor>,
            limit: usize,
        ) -> Result<crate::audit::AuditPage, AuditError> {
            self.inner.history(after, limit).await
        }

        async fn tip(&self) -> Result<String, AuditError> {
            self.inner.tip().await
        }

        async fn seals(&self) -> Result<Vec<crate::audit::SegmentSeal>, AuditError> {
            self.inner.seals().await
        }

        async fn verify(&self) -> Result<(), AuditError> {
            self.inner.verify().await
        }

        async fn rotate(&self) -> Result<crate::audit::SegmentSeal, AuditError> {
            self.inner.rotate().await
        }

        async fn close(&self) -> Result<crate::audit::SegmentSeal, AuditError> {
            self.inner.close().await
        }
    }

    /// Deterministic, generated (not checked in as a file): the generator is the
    /// fixture, and it reproduces byte-for-byte on every run, which is what "same
    /// corpus for every run" requires. Each document carries one regex-detectable
    /// email so the PII stage does real detection and redaction work, not a no-op.
    const CORPUS_DOC_COUNT: usize = 300;

    fn fixed_corpus() -> Vec<String> {
        (0..CORPUS_DOC_COUNT)
            .map(|i| {
                format!(
                    "Dear team, this is quarterly note number {i}. Contact person{i}@example.com \
                     for follow-up. This message intentionally repeats standard boilerplate text \
                     so that every document in the corpus stays close in size to its neighbours. \
                     Regards, Automated Reporter {i}."
                )
            })
            .collect()
    }

    /// Reports D2's three numbers — total per-document wall time, time inside
    /// `append`, and time waiting for `io_order` alone — at `--concurrency` 1, 2, 4,
    /// and CPU count, against the fixed corpus above. Evaluated against §9: throughput
    /// must reach 2x at CPU count relative to concurrency 1, and `io_order` wait must
    /// stay under 20% of per-document wall time, or Phase 6's audit work is unblocked
    /// immediately per §9.
    ///
    /// `#[ignore]`d like `step8_sync_policy_timing` in `audit/store_file.rs`: this
    /// measures wall-clock on shared CI/dev hardware, so it does not belong in the
    /// default suite. Run with `cargo test task5_concurrency_and_contention_measurement
    /// -- --ignored --nocapture`.
    #[tokio::test(flavor = "multi_thread", worker_threads = 8)]
    #[ignore]
    async fn task5_concurrency_and_contention_measurement() {
        let corpus = fixed_corpus();
        let corpus_bytes: usize = corpus.iter().map(|d| d.len()).sum();

        let cpu_count = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(1);
        let mut levels: Vec<usize> = vec![1, 2, 4];
        if !levels.contains(&cpu_count) {
            levels.push(cpu_count);
        }

        println!(
            "\n=== Task 5 measurement: {} documents, {corpus_bytes} bytes, cpu_count={cpu_count} ===",
            corpus.len(),
        );

        let mut wall_at_one = None;

        for concurrency in levels {
            let dir = TempDir::new(&format!("task5-c{concurrency}"));
            let file_store = Arc::new(
                FileAuditStore::open_with_policy(
                    dir.path(),
                    NodeId::new("bench-node"),
                    "bench-config",
                    crate::audit::SyncPolicy::EveryBatch,
                )
                .unwrap()
                .with_contention_tracking(),
            );
            let timing_store = Arc::new(TimingAuditStore::new(Arc::clone(&file_store)));

            let mut config = pii_config();
            config.concurrency = concurrency;
            let facade = HaciendaFacade::with_stores(
                HaciendaConfig::default().with_pii(config),
                Some(Arc::clone(&timing_store) as Arc<dyn AuditStore>),
                None,
            )
            .unwrap();

            let inputs = corpus.iter().map(|t| text_input(t)).collect();

            let wall_start = std::time::Instant::now();
            facade.process_batch(inputs).await.unwrap();
            let wall = wall_start.elapsed();

            let append_time = timing_store.total_append_time();
            let (io_order_wait, _) = file_store.io_order_wait().unwrap();
            facade.close().await.ok();

            if concurrency == 1 {
                wall_at_one = Some(wall);
            }
            let speedup = wall_at_one
                .map(|base| base.as_secs_f64() / wall.as_secs_f64())
                .unwrap_or(1.0);
            let per_doc_wall = wall / CORPUS_DOC_COUNT as u32;
            let wait_fraction = io_order_wait.as_secs_f64() / wall.as_secs_f64();

            println!(
                "concurrency={concurrency:<2} wall={wall:>10?} speedup={speedup:>5.2}x \
                 per_doc={per_doc_wall:>10?} append_time={append_time:>10?} \
                 io_order_wait={io_order_wait:>10?} wait_fraction={wait_fraction:.4}"
            );
        }

        // RAM headroom: a throughput curve measured under memory pressure measures
        // the memory, not the code, so it is recorded alongside the numbers above
        // rather than left implicit.
        if let Ok(meminfo) = std::fs::read_to_string("/proc/meminfo") {
            if let Some(line) = meminfo.lines().find(|l| l.starts_with("MemAvailable:")) {
                println!("RAM headroom at measurement time: {line}");
            }
        }
    }
}
