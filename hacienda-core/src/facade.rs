use xberg::{extract, ExtractInput, ExtractionConfig, ExtractionResult};
use hacienda_core::pii::{PiiPipeline, PipelineConfig, PipelineResult};
use hacienda_core::compliance::ComplianceGenerator;
use hacienda_core::audit::{AuditChain, FileSink};
use hacienda_core::review::ReviewQueue;
use hacienda_core::glossary::{EntityGlossary, generate_markdown_links};
use std::sync::{Arc, Mutex};

pub struct HaciendaFacade {
    extraction_config: ExtractionConfig,
    pii_pipeline: Option<PiiPipeline>,
    compliance: Option<ComplianceGenerator>,
    audit_chain: Option<Arc<Mutex<AuditChain>>>,
    review_queue: Option<Arc<ReviewQueue>>,
    glossary: Option<Arc<Mutex<EntityGlossary>>>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct HaciendaFacadeConfig {
    pub extraction: ExtractionConfig,
    pub pii: Option<PipelineConfig>,
    pub compliance: Option<compliance::ComplianceConfig>,
    pub audit: Option<audit::AuditConfig>,
    pub review: Option<review::ReviewConfig>,
    pub glossary: Option<glossary::GlossaryConfig>,
}

impl HaciendaFacade {
    pub fn new(config: HaciendaFacadeConfig) -> Result<Self, HaciendaError> {
        let pii_pipeline = config.pii.as_ref()
            .map(|c| PiiPipeline::new(c.clone()))
            .transpose()?;

        let compliance = config.compliance.as_ref()
            .map(|c| ComplianceGenerator::new(c.model_name.clone()));

        let audit_chain = config.audit.as_ref()
            .map(|c| Arc::new(Mutex::new(AuditChain::new(c.config_hash.clone()))));

        let review_queue = config.review.as_ref()
            .map(|c| Arc::new(ReviewQueue::new(c.clone())));

        let glossary = config.glossary.as_ref()
            .map(|c| Arc::new(Mutex::new(EntityGlossary::new(c.clone()))));

        // Register PII processor with xberg if PII enabled
        if let Some(pii_config) = &config.pii {
            hacienda_core::pii::xberg_integration::register_hacienda_pii(pii_config.clone())?;
        }

        Ok(Self {
            extraction_config: config.extraction,
            pii_pipeline,
            compliance,
            audit_chain,
            review_queue,
            glossary,
        })
    }

    /// Single-shot: extract → PII → compliance → audit → review → glossary
    pub async fn process(&self, input: ExtractInput) -> Result<HaciendaResult, HaciendaError> {
        let start = std::time::Instant::now();

        // 1. Extract (97 formats)
        let extraction = extract(input, &self.extraction_config).await?;

        // 2. PII Pipeline (optional)
        let pii_result = if let Some(pipeline) = &self.pii_pipeline {
            let text = extraction.results.first().map(|r| r.content.as_str()).unwrap_or("");
            let result = pipeline.process(text)?;

            // Log to audit chain
            if let Some(chain) = &self.audit_chain {
                let mut guard = chain.lock().unwrap();
                for entity in &result.entities {
                    let entry = AuditEntry::from_pii_entity(entity, &result);
                    guard.append(entry)?;
                }
            }

            // Submit to review queue if high-risk
            if let Some(queue) = &self.review_queue {
                for entity in &result.entities {
                    if entity.confidence < 0.5 || entity.category == "Custom" {
                        let req = ReviewRequest::from_pii_entity(entity);
                        queue.submit(req);
                    }
                }
            }

            Some(result)
        } else { None };

        // 3. Compliance (optional)
        let compliance_report = if let Some(comp) = &self.compliance {
            Some(comp.full_report().await?)
        } else { None };

        // 4. Glossary linking (optional)
        let glossary_links = if let Some(glossary) = &self.glossary {
            let mut guard = glossary.lock().unwrap();
            for entity in pii_result.as_ref().into_iter().flat_map(|r| &r.entities) {
                guard.insert(entity);
            }
            Some(guard.generate_links(&extraction.results[0].content)?)
        } else { None };

        Ok(HaciendaResult {
            extraction,
            pii: pii_result,
            compliance: compliance_report,
            audit_log: self.audit_chain.as_ref().map(|c| c.lock().unwrap().entries().to_vec()),
            review_count: self.review_queue.as_ref().map(|q| q.stats().pending).unwrap_or(0),
            glossary_links,
            metadata: HaciendaMetadata {
                processing_time_ms: start.elapsed().as_millis() as u64,
                pii_enabled: self.pii_pipeline.is_some(),
            },
        })
    }

    /// Batch processing
    pub async fn process_batch(&self, inputs: Vec<ExtractInput>) -> Result<Vec<HaciendaResult>, HaciendaError> {
        let mut results = Vec::with_capacity(inputs.len());
        for input in inputs {
            results.push(self.process(input).await?);
        }
        Ok(results)
    }
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct HaciendaResult {
    pub extraction: ExtractionResult,
    pub pii: Option<PipelineResult>,
    pub compliance: Option<ComplianceReport>,
    pub audit_log: Option<Vec<AuditEntry>>,
    pub review_count: usize,
    pub glossary_links: Option<String>,
    pub metadata: HaciendaMetadata,
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct HaciendaMetadata {
    pub processing_time_ms: u64,
    pub pii_enabled: bool,
}

mod compliance {
    use hacienda_core::compliance::{ComplianceGenerator, ComplianceReport, ComplianceChecklist, ModelCard, DoraReport};
    pub use hacienda_core::compliance::*;
}

mod audit {
    use hacienda_core::audit::{AuditChain, FileSink, AuditEntry, AuditSink};
    pub use hacienda_core::audit::*;
}

mod review {
    use hacienda_core::review::{ReviewQueue, ReviewQueueItem, ReviewRequest, ReviewDecision, ReviewStatus, Priority, QueueStats};
    pub use hacienda_core::review::*;
}

mod glossary {
    use hacienda_core::glossary::{EntityGlossary, GlossaryEntry, generate_markdown_links};
    pub use hacienda_core::glossary::*;
}

mod pii {
    use hacienda_core::pii::{PiiPipeline, PipelineConfig, PipelineResult, PipelineEntity, PipelineAuditEntry, PipelineMetrics};
    pub use hacienda_core::pii::*;
}