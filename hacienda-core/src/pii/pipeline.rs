use pii_pipeline::{PiiPipeline, PipelineConfig, PipelineResult, PipelineEntity, PipelineAuditEntry, PipelineMetrics};
use hacienda_core::redaction::{RedactionEngine, RedactionConfig, RedactionMode};
use hacienda_core::pii::profiles::RedactionProfile;
use std::sync::Arc;

pub struct PiiPipelineWrapper {
    inner: Arc<PiiPipeline>,
    config: PipelineConfig,
}

impl PiiPipelineWrapper {
    pub fn new(config: PipelineConfig) -> Result<Self, String> {
        let inner = PiiPipeline::new(config.clone())
            .map_err(|e| format!("Failed to create PiiPipeline: {}", e))?;
        Ok(Self {
            inner: Arc::new(inner),
            config,
        })
    }

    pub fn process(&self, text: &str) -> Result<PipelineResult, String> {
        self.inner.process(text)
    }
}

pub use pii_pipeline::{PipelineResult, PipelineEntity, PipelineAuditEntry, PipelineMetrics};
pub use pii_config::PipelineConfig;