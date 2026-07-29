//! One call from a document to redacted text, an audit trail, and compliance artefacts.

use crate::audit::{AuditEntry, AuditEntryInput, AuditStore, InMemoryAuditStore};
use crate::compliance::{ComplianceGenerator, ComplianceReport};
use crate::config::HaciendaConfig;
use crate::error::HaciendaError;
use crate::glossary::{EntityGlossary, GlossaryEntry};
use crate::pii::{PiiError, PiiPipeline, PipelineResult};
use crate::redaction::{KeyId, KeyResolver, Pseudonymiser, RedactionError};
use crate::review::store::ReviewStore;
use crate::review::{ReviewConfig, ReviewQueue, ReviewRequest};
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};
use xberg::{extract, ExtractInput, ExtractionResult};

/// Version recorded on every audit entry so a record can be tied to the code that made it.
const PIPELINE_VERSION: &str = env!("CARGO_PKG_VERSION");

pub struct HaciendaFacade {
    config: HaciendaConfig,
    pii_pipeline: Option<PiiPipeline>,
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
            .transpose()?;

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

    /// A snapshot of the open segment's audit entries.
    ///
    /// Returns only the open segment. For the full history across sealed segments,
    /// call the store's `seals()` and re-read the segment files directly.
    ///
    /// Returns an empty `Vec` when auditing is not configured — callers do not need to
    /// distinguish "no store" from "store with no entries yet".
    pub async fn audit_entries(&self) -> Result<Vec<AuditEntry>, HaciendaError> {
        match &self.audit_store {
            Some(store) => Ok(store.entries().await?),
            None => Ok(Vec::new()),
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

    /// Process several inputs as one extraction, sharing the audit chain and glossary.
    ///
    /// # Errors
    ///
    /// As [`HaciendaFacade::process`].
    pub async fn process_batch(
        &self,
        inputs: Vec<ExtractInput>,
    ) -> Result<HaciendaResult, HaciendaError> {
        let start = std::time::Instant::now();

        let mut extraction = extract_all(inputs, &self.config).await?;

        let mut pii = Vec::new();
        let mut audit_entries = Vec::new();
        let mut review_submitted = 0;

        if let Some(pipeline) = &self.pii_pipeline {
            for document in &mut extraction.results {
                let result = pipeline.process(&document.content).await?;

                self.observe_glossary(&document.content, &result);
                audit_entries.extend(self.record_audit(&result).await?);
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
    ) -> Result<Vec<AuditEntry>, HaciendaError> {
        let Some(store) = &self.audit_store else {
            return Ok(Vec::new());
        };

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
    use std::sync::atomic::{AtomicUsize, Ordering};

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
}
