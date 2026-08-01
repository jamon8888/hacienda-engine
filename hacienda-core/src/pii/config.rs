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
    /// How many documents [`crate::HaciendaFacade::process_batch_with_auth`] runs
    /// through this pipeline at once.
    ///
    /// `1` is the default and preserves the pipeline's original strictly-sequential
    /// behaviour. Raising it bounds a worker pool over the batch's documents — it does
    /// not change what is returned: results still come back in input order (see
    /// `HaciendaResult::pii`) and every document is still audited and reviewed exactly
    /// once, regardless of completion order.
    pub concurrency: usize,
    /// The zero-shot label extension for this pipeline, if one is configured.
    ///
    /// `None` runs the base five categories only. This field is deliberately not yet
    /// threaded into detection — see `VerticalConfig`'s own docs.
    pub vertical: Option<VerticalConfig>,
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
            concurrency: 1,
            vertical: None,
        }
    }
}

/// A schema-level NER specialisation: a stable identifier plus the zero-shot labels it
/// hands the NER backend in addition to the base categories.
///
/// A vertical is a **label set**, not a model or a trained adapter — see the
/// `vertical-lora-training` project convention. This type is currently config-schema
/// only: nothing in the pipeline reads it yet. Threading it into detection is separate,
/// gated work (see
/// `superpowers/plans/2026-07-31-vertical-model-specialisation-implementation.md` Task
/// 2.2), so constructing a `PipelineConfig` with `vertical: Some(..)` today changes
/// nothing about what is detected.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct VerticalConfig {
    /// Stable identifier recorded in the audit chain.
    pub id: String,
    /// Zero-shot labels handed to the NER backend in addition to the base categories.
    pub labels: Vec<String>,
}

impl VerticalConfig {
    /// Validate the vertical's shape independent of how (or whether) it is used.
    ///
    /// This does not depend on any detector, model, or redaction mode — a malformed
    /// vertical is a configuration error in every build profile.
    ///
    /// # Errors
    ///
    /// Returns [`PiiError::InvalidVertical`] when:
    /// - `id` is empty (after trimming),
    /// - `labels` is empty,
    /// - any label is empty after trimming whitespace,
    /// - any label contains `[`, `:`, or `]` (the pseudonym token's delimiters — see
    ///   `category_label` in `crate::redaction::pseudonym`, which this mirrors), or
    /// - two labels are equal after case-folding.
    pub fn validate(&self) -> Result<(), PiiError> {
        if self.id.trim().is_empty() {
            return Err(PiiError::InvalidVertical {
                id: self.id.clone(),
                reason: "id must not be empty".to_string(),
            });
        }
        if self.labels.is_empty() {
            return Err(PiiError::InvalidVertical {
                id: self.id.clone(),
                reason: "labels must not be empty".to_string(),
            });
        }

        let mut seen = std::collections::HashSet::with_capacity(self.labels.len());
        for label in &self.labels {
            if label.trim().is_empty() {
                return Err(PiiError::InvalidVertical {
                    id: self.id.clone(),
                    reason: format!("label '{label}' is empty"),
                });
            }
            if label.contains('[') || label.contains(':') || label.contains(']') {
                return Err(PiiError::InvalidVertical {
                    id: self.id.clone(),
                    reason: format!("label '{label}' contains a token delimiter ('[', ':' or ']')"),
                });
            }
            if !seen.insert(label.to_lowercase()) {
                return Err(PiiError::InvalidVertical {
                    id: self.id.clone(),
                    reason: format!("label '{label}' is a duplicate (case-insensitive)"),
                });
            }
        }

        Ok(())
    }

    /// The value recorded in the audit chain's `vertical` field: `"<id>@<digest>"`, where
    /// `digest` is the first 8 hex characters of the blake3 hash of the sorted,
    /// case-folded label set.
    ///
    /// An id alone would be a false provenance claim — the same id with a different label
    /// set detects different things, and the audit record's job is to say what *was*
    /// detectable. The digest makes a silently-edited label set visible without recording
    /// the labels themselves in every entry. Sorting and case-folding before hashing means
    /// reordering or re-casing the label list in config does not spuriously change every
    /// subsequent entry's provenance value.
    pub fn provenance_id(&self) -> String {
        let mut labels: Vec<String> = self
            .labels
            .iter()
            .map(|label| label.to_lowercase())
            .collect();
        labels.sort_unstable();

        let mut hasher = blake3::Hasher::new();
        for label in &labels {
            hasher.update(label.as_bytes());
            hasher.update(b"\0");
        }
        let digest = hasher.finalize().to_hex().to_string();

        format!("{}@{}", self.id, &digest[..8])
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
        assert_eq!(
            config.concurrency, 1,
            "default concurrency must preserve the original sequential behaviour"
        );
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

    #[test]
    fn should_default_to_no_vertical() {
        assert_eq!(PipelineConfig::default().vertical, None);
    }

    fn finance_vertical() -> VerticalConfig {
        VerticalConfig {
            id: "finance".to_string(),
            labels: vec!["iban".to_string(), "swift_code".to_string()],
        }
    }

    #[test]
    fn should_accept_a_well_formed_vertical() {
        assert!(finance_vertical().validate().is_ok());
    }

    #[test]
    fn should_reject_an_empty_id() {
        let vertical = VerticalConfig {
            id: String::new(),
            ..finance_vertical()
        };
        let error = vertical.validate().unwrap_err();
        assert!(matches!(error, PiiError::InvalidVertical { .. }));
        assert!(error.to_string().contains("id must not be empty"));
    }

    #[test]
    fn should_reject_a_whitespace_only_id() {
        let vertical = VerticalConfig {
            id: "   ".to_string(),
            ..finance_vertical()
        };
        assert!(matches!(
            vertical.validate(),
            Err(PiiError::InvalidVertical { .. })
        ));
    }

    #[test]
    fn should_reject_empty_labels() {
        let vertical = VerticalConfig {
            id: "finance".to_string(),
            labels: vec![],
        };
        let error = vertical.validate().unwrap_err();
        assert!(matches!(error, PiiError::InvalidVertical { .. }));
        assert!(error.to_string().contains("labels must not be empty"));
    }

    #[test]
    fn should_reject_a_whitespace_only_label() {
        let vertical = VerticalConfig {
            id: "finance".to_string(),
            labels: vec!["   ".to_string()],
        };
        let error = vertical.validate().unwrap_err();
        assert!(matches!(error, PiiError::InvalidVertical { .. }));
        assert!(error.to_string().contains("is empty"));
    }

    #[test]
    fn should_reject_a_label_containing_an_open_bracket() {
        let vertical = VerticalConfig {
            id: "finance".to_string(),
            labels: vec!["iban[".to_string()],
        };
        let error = vertical.validate().unwrap_err();
        assert!(matches!(error, PiiError::InvalidVertical { .. }));
        assert!(error.to_string().contains("token delimiter"));
    }

    #[test]
    fn should_reject_a_label_containing_a_colon() {
        let vertical = VerticalConfig {
            id: "finance".to_string(),
            labels: vec!["iban:code".to_string()],
        };
        let error = vertical.validate().unwrap_err();
        assert!(matches!(error, PiiError::InvalidVertical { .. }));
        assert!(error.to_string().contains("token delimiter"));
    }

    #[test]
    fn should_reject_a_label_containing_a_close_bracket() {
        let vertical = VerticalConfig {
            id: "finance".to_string(),
            labels: vec!["iban]".to_string()],
        };
        let error = vertical.validate().unwrap_err();
        assert!(matches!(error, PiiError::InvalidVertical { .. }));
        assert!(error.to_string().contains("token delimiter"));
    }

    #[test]
    fn should_reject_duplicate_labels_after_case_folding() {
        let vertical = VerticalConfig {
            id: "finance".to_string(),
            labels: vec!["IBAN".to_string(), "iban".to_string()],
        };
        let error = vertical.validate().unwrap_err();
        assert!(matches!(error, PiiError::InvalidVertical { .. }));
        assert!(error.to_string().contains("duplicate"));
    }

    #[test]
    fn should_compute_a_stable_provenance_id_for_well_formed_input() {
        let id = finance_vertical().provenance_id();
        assert!(id.starts_with("finance@"));
        assert_eq!(id.len(), "finance@".len() + 8);
    }

    #[test]
    fn should_produce_the_same_provenance_id_regardless_of_label_order() {
        let a = VerticalConfig {
            id: "finance".to_string(),
            labels: vec!["iban".to_string(), "swift_code".to_string()],
        };
        let b = VerticalConfig {
            id: "finance".to_string(),
            labels: vec!["swift_code".to_string(), "iban".to_string()],
        };
        assert_eq!(a.provenance_id(), b.provenance_id());
    }

    #[test]
    fn should_produce_the_same_provenance_id_regardless_of_label_case() {
        let a = VerticalConfig {
            id: "finance".to_string(),
            labels: vec!["IBAN".to_string(), "swift_code".to_string()],
        };
        let b = VerticalConfig {
            id: "finance".to_string(),
            labels: vec!["iban".to_string(), "SWIFT_code".to_string()],
        };
        assert_eq!(a.provenance_id(), b.provenance_id());
    }

    #[test]
    fn should_change_the_provenance_id_when_a_label_is_added() {
        let base = finance_vertical();
        let mut extended = base.clone();
        extended.labels.push("account_number".to_string());
        assert_ne!(base.provenance_id(), extended.provenance_id());
    }

    #[test]
    fn should_change_the_provenance_id_when_the_id_changes() {
        let a = finance_vertical();
        let b = VerticalConfig {
            id: "legal".to_string(),
            ..finance_vertical()
        };
        assert_ne!(a.provenance_id(), b.provenance_id());
    }
}
