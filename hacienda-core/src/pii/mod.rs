mod pipeline;
mod config;
mod xberg_integration;

pub use pipeline::*;
pub use config::*;
pub use xberg_integration::*;

use pii_pipeline::PiiPipeline;
use pii_config::PipelineConfig as PiiPipelineConfig;

pub struct PiiPipelineWrapper {
    inner: PiiPipeline,
}

impl PiiPipelineWrapper {
    pub fn new(config: PipelineConfig) -> Result<Self, String> {
        let pii_config: PiiPipelineConfig = config.into();
        let inner = PiiPipeline::new(pii_config)
            .map_err(|e| format!("Failed to create PII pipeline: {}", e))?;
        Ok(Self { inner })
    }

    pub fn process(&self, text: &str) -> Result<PipelineResult, String> {
        self.inner.process(text)
            .map_err(|e| e.to_string())
    }
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct PipelineResult {
    pub redacted_text: String,
    pub entities: Vec<PipelineEntity>,
    pub audit_log: Vec<PipelineAuditEntry>,
    pub metrics: PipelineMetrics,
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct PipelineEntity {
    pub category: String,
    pub start: u32,
    pub end: u32,
    pub confidence: f32,
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct PipelineAuditEntry {
    pub category: String,
    pub span_hash: String,
    pub chain_hash: String,
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct PipelineMetrics {
    pub regex_ms: u64,
    pub model_ms: u64,
    pub merge_ms: u64,
    pub redaction_ms: u64,
    pub total_ms: u64,
    pub entities_detected: u32,
    pub entities_redacted: u32,
}

impl From<pii_pipeline::PipelineResult> for PipelineResult {
    fn from(r: pii_pipeline::PipelineResult) -> Self {
        Self {
            redacted_text: r.redacted_text,
            entities: r.entities.into_iter().map(|e| PipelineEntity {
                category: e.category,
                start: e.start,
                end: e.end,
                confidence: e.confidence,
            }).collect(),
            audit_log: r.audit_log.into_iter().map(|e| PipelineAuditEntry {
                category: e.category,
                span_hash: e.span_hash,
                chain_hash: e.chain_hash,
            }).collect(),
            metrics: PipelineMetrics {
                regex_ms: r.metrics.regex_ms,
                model_ms: r.metrics.model_ms,
                merge_ms: r.metrics.merge_ms,
                redaction_ms: r.metrics.redaction_ms,
                total_ms: r.metrics.total_ms,
                entities_detected: r.metrics.entities_detected,
                entities_redacted: r.metrics.entities_redacted,
            },
        }
    }
}

impl From<hacienda_core::config::pii::PipelineConfig> for pii_config::PipelineConfig {
    fn from(c: hacienda_core::config::pii::PipelineConfig) -> Self {
        Self {
            regex_first: c.regex_first,
            model_threshold_default: c.model_threshold_default,
            merge_overlap_threshold: c.merge_overlap_threshold,
            redaction: c.redaction.into(),
            audit: c.audit.into(),
            model: c.model.into(),
        }
    }
}

impl From<hacienda_core::config::pii::RedactionProfile> for pii_config::RedactionConfig {
    fn from(p: hacienda_core::config::pii::RedactionProfile) -> Self {
        match p {
            hacienda_core::config::pii::RedactionProfile::Default => Self::default(),
            hacienda_core::config::pii::RedactionProfile::PCI => Self {
                mode: "pseudonymize".into(),
                fpe_key_b64: None,
                custom_template: None,
                preserve_format: true,
            },
            hacienda_core::config::pii::RedactionProfile::HIPAA => Self {
                mode: "remove".into(),
                fpe_key_b64: None,
                custom_template: None,
                preserve_format: false,
            },
            hacienda_core::config::pii::RedactionProfile::GDPR => Self {
                mode: "pseudonymize".into(),
                fpe_key_b64: None,
                custom_template: None,
                preserve_format: true,
            },
            hacienda_core::config::pii::RedactionProfile::Custom(c) => Self {
                mode: "custom".into(),
                fpe_key_b64: None,
                custom_template: Some(c.patterns.join("|")),
                preserve_format: true,
            },
        }
    }
}

impl From<hacienda_core::config::pii::ModelConfig> for pii_config::ModelConfig {
    fn from(c: hacienda_core::config::pii::ModelConfig) -> Self {
        Self {
            enabled: c.enabled,
            model_id: c.model_id,
            revision: c.revision,
            device: c.device,
            dtype: c.dtype,
            thresholds_file: c.thresholds_file,
            max_seq_len: c.max_seq_len,
            batch_size: c.batch_size,
        }
    }
}

impl From<hacienda_core::config::audit::AuditConfig> for pii_config::AuditConfig {
    fn from(c: hacienda_core::config::audit::AuditConfig) -> Self {
        Self {
            enabled: c.enabled,
            log_path: c.log_path,
            format: c.format,
            rotation: c.rotation,
            max_files: c.max_files,
            max_file_size_mb: c.max_file_size_mb,
            include_span_hash: c.include_span_hash,
            hash_algorithm: c.hash_algorithm,
        }
    }
}