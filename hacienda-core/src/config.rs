use serde::{Deserialize, Serialize};
use xberg::ExtractionConfig;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HaciendaConfig {
    pub extraction: ExtractionConfig,
    pub pii: Option<pii::PipelineConfig>,
    pub compliance: Option<compliance::ComplianceConfig>,
    pub audit: Option<audit::AuditConfig>,
    pub review: Option<review::ReviewConfig>,
    pub glossary: Option<glossary::GlossaryConfig>,
}

impl Default for HaciendaConfig {
    fn default() -> Self {
        Self {
            extraction: ExtractionConfig::default(),
            pii: None,
            compliance: None,
            audit: None,
            review: None,
            glossary: None,
        }
    }
}

pub mod pii {
    use serde::{Deserialize, Serialize};

    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct PipelineConfig {
        pub regex_first: bool,
        pub model_threshold_default: f32,
        pub merge_overlap_threshold: f32,
        pub redaction: RedactionProfile,
        pub audit: super::audit::AuditConfig,
        pub model: ModelConfig,
    }

    impl Default for PipelineConfig {
        fn default() -> Self {
            Self {
                regex_first: true,
                model_threshold_default: 0.5,
                merge_overlap_threshold: 0.5,
                redaction: RedactionProfile::default(),
                audit: super::audit::AuditConfig::default(),
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
}

pub mod compliance {
    use serde::{Deserialize, Serialize};

    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct ComplianceConfig {
        pub model_name: String,
        pub enabled_reports: Vec<ReportType>,
    }

    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub enum ReportType {
        DPIA,
        ModelCard,
        DORA,
        AIAct,
        Checklist,
    }
}

pub mod audit {
    use serde::{Deserialize, Serialize};

    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct AuditConfig {
        pub enabled: bool,
        pub log_path: String,
        pub format: String,
        pub rotation: String,
        pub max_files: usize,
        pub max_file_size_mb: u64,
        pub include_span_hash: bool,
        pub hash_algorithm: String,
        pub config_hash: String,
    }

    impl Default for AuditConfig {
        fn default() -> Self {
            Self {
                enabled: true,
                log_path: "audit.log".into(),
                format: "jsonl".into(),
                rotation: "daily".into(),
                max_files: 30,
                max_file_size_mb: 100,
                include_span_hash: true,
                hash_algorithm: "blake3".into(),
                config_hash: "default".into(),
            }
        }
    }
}

pub mod review {
    use serde::{Deserialize, Serialize};

    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct ReviewConfig {
        pub auto_assign: bool,
        pub default_priority: Priority,
        pub deadline_hours: Option<u64>,
    }

    impl Default for ReviewConfig {
        fn default() -> Self {
            Self {
                auto_assign: false,
                default_priority: Priority::High,
                deadline_hours: Some(24),
            }
        }
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
    #[serde(rename_all = "snake_case")]
    pub enum Priority {
        Low,
        Normal,
        High,
        Critical,
    }

    impl Default for Priority {
        fn default() -> Self {
            Self::High
        }
    }
}

pub mod glossary {
    use serde::{Deserialize, Serialize};

    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct GlossaryConfig {
        pub enabled: bool,
        pub link_style: LinkStyle,
        pub min_confidence: f32,
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
    #[serde(rename_all = "snake_case")]
    pub enum LinkStyle {
        Markdown,
        Html,
        Wiki,
    }

    impl Default for GlossaryConfig {
        fn default() -> Self {
            Self {
                enabled: true,
                link_style: LinkStyle::Markdown,
                min_confidence: 0.5,
            }
        }
    }
}