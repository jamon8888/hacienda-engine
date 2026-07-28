use crate::redaction::RedactionConfig;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PipelineConfig {
    pub regex_first: bool,
    pub model_threshold_default: f32,
    pub merge_overlap_threshold: f32,
    pub redaction: RedactionProfile,
    pub audit: crate::audit::AuditConfig,
    pub model: ModelConfig,
}

impl Default for PipelineConfig {
    fn default() -> Self {
        Self {
            regex_first: true,
            model_threshold_default: 0.5,
            merge_overlap_threshold: 0.5,
            redaction: RedactionProfile::default(),
            audit: crate::audit::AuditConfig::default(),
            model: ModelConfig::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelConfig {
    pub enabled: bool,
    pub model_id: String,
    pub revision: String,
    pub device: String,
    pub dtype: String,
    pub thresholds_file: Option<String>,
    pub max_seq_len: u32,
    pub batch_size: u32,
}

impl Default for ModelConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            model_id: "fastino/GLiNER2-Guardrails-PII-Multi".into(),
            revision: "main".into(),
            device: "cpu".into(),
            dtype: "f16".into(),
            thresholds_file: None,
            max_seq_len: 512,
            batch_size: 32,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum RedactionProfile {
    Default,
    PCI,
    HIPAA,
    GDPR,
    Custom(CustomProfile),
}

impl Default for RedactionProfile {
    fn default() -> Self {
        Self::Default
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CustomProfile {
    pub patterns: Vec<String>,
    pub terms: Vec<String>,
}
