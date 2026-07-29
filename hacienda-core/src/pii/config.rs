//! Pipeline configuration, loaded from TOML and overridable from a CLI.

use crate::audit::AuditConfig;
use crate::pii::PiiError;
use crate::redaction::{RedactionConfig, RedactionMode};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Effective configuration for one [`crate::pii::PiiPipeline`].
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct PipelineConfig {
    /// Prefer deterministic regex spans over model spans when the two overlap.
    pub regex_first: bool,
    /// Model detections scoring below this are discarded before merging.
    pub model_threshold_default: f32,
    /// Overlap ratio above which two spans are treated as the same detection.
    pub merge_overlap_threshold: f32,
    pub redaction: RedactionConfig,
    pub audit: AuditConfig,
    pub model: ModelConfig,
}

impl Default for PipelineConfig {
    fn default() -> Self {
        Self {
            regex_first: true,
            model_threshold_default: 0.5,
            merge_overlap_threshold: 0.5,
            redaction: RedactionConfig::default(),
            audit: AuditConfig::default(),
            model: ModelConfig::default(),
        }
    }
}

/// Which NER model the statistical half of the pipeline runs, if any.
///
/// The model is loaded from a local directory rather than a hub id: detection runs
/// on-premise and must not reach the network at inference time.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ModelConfig {
    /// When false the pipeline is regex-only. Nothing is loaded.
    pub enabled: bool,
    /// Directory holding the GLiNER2 base weights, tokenizer, and config.
    pub model_dir: Option<PathBuf>,
    /// Directory holding a PEFT LoRA adapter to merge into the base weights at load.
    pub lora_adapter_dir: Option<PathBuf>,
}

/// Values supplied on the command line, applied on top of the loaded file.
#[derive(Debug, Clone, Default)]
pub struct CliOverrides {
    pub model_threshold: Option<f32>,
    pub redaction_mode: Option<RedactionMode>,
    pub model_dir: Option<PathBuf>,
    pub lora_adapter_dir: Option<PathBuf>,
}

impl PipelineConfig {
    /// Load configuration, falling back to defaults when `config_path` is `None`.
    ///
    /// Precedence is defaults < file < `overrides`.
    ///
    /// # Errors
    ///
    /// Returns [`PiiError::ConfigIo`] if the file cannot be read and
    /// [`PiiError::ConfigParse`] if it is not valid TOML for this schema.
    pub fn load(config_path: Option<&Path>, overrides: CliOverrides) -> Result<Self, PiiError> {
        let mut config = match config_path {
            Some(path) => Self::from_file(path)?,
            None => Self::default(),
        };
        config.apply(overrides);
        Ok(config)
    }

    /// Read and parse a TOML configuration file.
    ///
    /// # Errors
    ///
    /// Returns [`PiiError::ConfigIo`] or [`PiiError::ConfigParse`].
    pub fn from_file(path: &Path) -> Result<Self, PiiError> {
        let content = std::fs::read_to_string(path).map_err(|source| PiiError::ConfigIo {
            path: path.display().to_string(),
            source,
        })?;
        toml::from_str(&content).map_err(|source| PiiError::ConfigParse {
            path: path.display().to_string(),
            source,
        })
    }

    /// Overlay command-line values. Absent overrides leave the field untouched.
    ///
    /// Supplying `model_dir` also enables the model: naming a model and then
    /// silently not running it is never what the caller meant.
    pub fn apply(&mut self, overrides: CliOverrides) {
        if let Some(threshold) = overrides.model_threshold {
            self.model_threshold_default = threshold;
        }
        if let Some(mode) = overrides.redaction_mode {
            self.redaction.mode = mode;
        }
        if let Some(dir) = overrides.model_dir {
            self.model.model_dir = Some(dir);
            self.model.enabled = true;
        }
        if let Some(dir) = overrides.lora_adapter_dir {
            self.model.lora_adapter_dir = Some(dir);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn should_default_to_a_regex_only_pipeline() {
        let config = PipelineConfig::default();
        assert!(config.regex_first);
        assert!(!config.model.enabled);
        assert!(config.model.model_dir.is_none());
    }

    #[test]
    fn should_use_defaults_when_no_file_is_given() {
        let config = PipelineConfig::load(None, CliOverrides::default()).unwrap();
        assert_eq!(config.merge_overlap_threshold, 0.5);
    }

    #[test]
    fn should_apply_cli_overrides_over_the_loaded_values() {
        let config = PipelineConfig::load(
            None,
            CliOverrides {
                model_threshold: Some(0.9),
                redaction_mode: Some(RedactionMode::Hash),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(config.model_threshold_default, 0.9);
        assert_eq!(config.redaction.mode, RedactionMode::Hash);
    }

    #[test]
    fn should_enable_the_model_when_a_model_directory_is_supplied() {
        let mut config = PipelineConfig::default();
        config.apply(CliOverrides {
            model_dir: Some(PathBuf::from("/models/gliner2")),
            ..Default::default()
        });
        assert!(config.model.enabled);
        assert_eq!(
            config.model.model_dir.unwrap(),
            PathBuf::from("/models/gliner2")
        );
    }

    #[test]
    fn should_parse_a_partial_toml_file_over_the_defaults() {
        let config: PipelineConfig = toml::from_str("regex_first = false\n").unwrap();
        assert!(!config.regex_first);
        // Untouched sections keep their defaults rather than failing to deserialize.
        assert_eq!(config.redaction.mode, RedactionMode::default());
        assert_eq!(config.redaction.mode, RedactionMode::Mask);
    }

    #[test]
    fn should_report_the_path_when_a_config_file_is_missing() {
        let error = PipelineConfig::from_file(Path::new("/nonexistent/hacienda.toml")).unwrap_err();
        assert!(matches!(error, PiiError::ConfigIo { .. }));
        assert!(error.to_string().contains("/nonexistent/hacienda.toml"));
    }
}
