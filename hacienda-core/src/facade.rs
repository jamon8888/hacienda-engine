//! One call from a document to redacted text, an audit trail, and compliance artefacts.

use crate::audit::{AuditChain, AuditEntry, AuditEntryInput};
use crate::compliance::{ComplianceGenerator, ComplianceReport};
use crate::config::HaciendaConfig;
use crate::error::HaciendaError;
use crate::glossary::{EntityGlossary, GlossaryEntry};
use crate::pii::{PiiPipeline, PipelineResult};
use crate::review::{ReviewQueue, ReviewRequest};
use serde::{Deserialize, Serialize};
use std::sync::Mutex;
use xberg::{extract, ExtractInput, ExtractionResult};

/// Version recorded on every audit entry so a record can be tied to the code that made it.
const PIPELINE_VERSION: &str = env!("CARGO_PKG_VERSION");

pub struct HaciendaFacade {
    config: HaciendaConfig,
    pii_pipeline: Option<PiiPipeline>,
    compliance: Option<ComplianceGenerator>,
    audit_chain: Option<Mutex<AuditChain>>,
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
    /// # Errors
    ///
    /// Returns [`HaciendaError::Pii`] if the detection pipeline cannot be built —
    /// most often because a model is enabled that this build cannot load.
    pub fn new(config: HaciendaConfig) -> Result<Self, HaciendaError> {
        let pii_pipeline = config.pii.clone().map(PiiPipeline::new).transpose()?;

        // Auditing without detection would record nothing, so the chain follows the
        // pipeline rather than being independently switchable.
        let audit_chain = config
            .pii
            .as_ref()
            .filter(|p| p.audit.enabled)
            .map(|p| Mutex::new(AuditChain::new(p.audit.config_hash.clone())));

        Ok(Self {
            compliance: config.compliance.clone().map(ComplianceGenerator::new),
            review_queue: config.review.clone().map(ReviewQueue::new),
            glossary: config
                .glossary
                .clone()
                .filter(|g| g.enabled)
                .map(|g| Mutex::new(EntityGlossary::new(g))),
            pii_pipeline,
            audit_chain,
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

    /// A snapshot of the audit chain, including entries from earlier calls.
    pub fn audit_entries(&self) -> Vec<AuditEntry> {
        self.audit_chain
            .as_ref()
            .map(|chain| lock(chain).entries().to_vec())
            .unwrap_or_default()
    }

    /// Verify the audit chain has not been tampered with.
    ///
    /// # Errors
    ///
    /// Returns [`HaciendaError::Audit`] naming the first entry whose hash does not
    /// match the chain.
    pub fn verify_audit(&self) -> Result<(), HaciendaError> {
        match &self.audit_chain {
            Some(chain) => Ok(lock(chain).verify()?),
            None => Ok(()),
        }
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
                audit_entries.extend(self.record_audit(&result));
                review_submitted += self.submit_for_review(&result);

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
            lock(glossary).observe(text, &result.entities);
        }
    }

    fn record_audit(&self, result: &PipelineResult) -> Vec<AuditEntry> {
        let Some(chain) = &self.audit_chain else {
            return Vec::new();
        };
        let config_hash = lock(chain).config_hash().to_string();
        let mut guard = lock(chain);

        result
            .audit_log
            .iter()
            .map(|entry| {
                guard
                    .push(AuditEntryInput {
                        id: uuid::Uuid::new_v4().to_string(),
                        category: entry.category.clone(),
                        action: entry.action.into(),
                        span_hash: entry.span_hash.clone(),
                        span_length: entry.span_length,
                        confidence: entry.confidence,
                        source: entry.source.into(),
                        pipeline_version: PIPELINE_VERSION.to_string(),
                        config_hash: config_hash.clone(),
                    })
                    .clone()
            })
            .collect()
    }

    fn submit_for_review(&self, result: &PipelineResult) -> usize {
        let Some(queue) = &self.review_queue else {
            return 0;
        };
        result
            .entities
            .iter()
            .filter(|entity| queue.needs_review(entity.confidence))
            .map(|entity| {
                queue.submit(ReviewRequest {
                    // The snippet is the model's own mention text, which is empty for
                    // regex spans — those are deterministic and need no human context.
                    text_snippet: entity.text.clone(),
                    category: entity.category.to_string(),
                    start: entity.start,
                    end: entity.end,
                    confidence: entity.confidence,
                    source: entity.source.to_string(),
                });
            })
            .count()
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
/// The protected state is an append-only chain and a counter map; neither can be left
/// half-written by a panic, so refusing to serve later requests buys nothing.
fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::glossary::GlossaryConfig;
    use crate::pii::PipelineConfig;
    use crate::redaction::{RedactionConfig, RedactionMode};
    use crate::review::ReviewConfig;

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
        facade.verify_audit().unwrap();
    }

    #[tokio::test]
    async fn should_carry_the_audit_chain_across_calls() {
        let facade = HaciendaFacade::new(HaciendaConfig::default().with_pii(pii_config())).unwrap();
        facade.process(text_input("bob@example.com")).await.unwrap();
        facade.process(text_input("amy@example.com")).await.unwrap();

        assert_eq!(facade.audit_entries().len(), 2);
        facade.verify_audit().unwrap();
    }

    #[tokio::test]
    async fn should_keep_no_audit_chain_when_auditing_is_disabled() {
        let mut config = pii_config();
        config.audit.enabled = false;
        let facade = HaciendaFacade::new(HaciendaConfig::default().with_pii(config)).unwrap();

        let result = facade.process(text_input("bob@example.com")).await.unwrap();
        assert!(result.audit_entries.is_empty());
        assert!(facade.audit_entries().is_empty());
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
        assert_eq!(facade.review_queue().unwrap().stats().pending, 0);
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
}
