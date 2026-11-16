//! Pipeline configuration, loaded from TOML and overridable from a CLI.

use crate::audit::AuditConfig;
use crate::pii::types::PiiCategory;
use crate::pii::PiiError;
use crate::redaction::{RedactionConfig, RedactionMode};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

/// Length, in hex characters, of the digest suffix in [`VerticalConfig::provenance_id`].
const PROVENANCE_DIGEST_LEN: usize = 8;

/// Base NER categories every pipeline already requests, regardless of vertical —
/// see [`crate::pii::ner::DEFAULT_CATEGORIES`]. A vertical label that
/// case-insensitively duplicates one of these asks the model for the same concept
/// twice under two different category representations.
const BASE_CATEGORY_NAMES: [&str; 5] = ["person", "organization", "location", "email", "phone"];

/// Effective configuration for one [`crate::pii::PiiPipeline`].
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct PipelineConfig {
    /// Prefer deterministic regex spans over model spans when the two overlap.
    pub regex_first: bool,
    /// Model detections scoring below this are discarded before merging.
    pub model_threshold_default: f32,
    /// Per-category threshold overrides — when a category is present here its value
    /// is used instead of `model_threshold_default`. Empty (default) means
    /// `effective_threshold` applies the hybrid persona-boost table.
    pub model_thresholds: HashMap<PiiCategory, f32>,
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
    /// The Tier 0 schema vertical bound to this pipeline, if any.
    ///
    /// `None` (the default) is a strict no-op: nothing about detection changes unless
    /// an operator explicitly writes a `[pii.vertical]` section. See
    /// `superpowers/specs/2026-07-31-vertical-model-specialisation-design.md` §4.1 and
    /// [`VerticalConfig`].
    ///
    /// There is deliberately no `verticals` array or selector — one optional vertical,
    /// bound at pipeline construction. A registry is premature abstraction at N=1; see
    /// the implementation plan's "Explicit Non-Goals".
    pub vertical: Option<VerticalConfig>,
}

impl Default for PipelineConfig {
    fn default() -> Self {
        Self {
            regex_first: true,
            model_threshold_default: 0.5,
            model_thresholds: HashMap::new(),
            merge_overlap_threshold: 0.5,
            redaction: RedactionConfig::default(),
            audit: AuditConfig::default(),
            model: ModelConfig::default(),
            concurrency: 1,
            vertical: None,
        }
    }
}

impl PipelineConfig {
    /// Effective threshold for a concrete `PiiCategory`, honouring overrides in
    /// `model_thresholds` or the hybrid persona-boost table otherwise.
    pub fn effective_threshold(&self, cat: &PiiCategory) -> f32 {
        if let Some(v) = self.model_thresholds.get(cat) {
            return *v;
        }
        match cat {
            PiiCategory::Person
            | PiiCategory::FullName
            | PiiCategory::FirstName
            | PiiCategory::MiddleName
            | PiiCategory::LastName => 0.65,
            PiiCategory::Email
            | PiiCategory::PhoneNumber
            | PiiCategory::Iban
            | PiiCategory::IpAddress
            | PiiCategory::CreditCard => 0.48,
            _ => self.model_threshold_default,
        }
    }
}

/// A Tier 0 "schema vertical": a named set of zero-shot labels handed to the NER
/// backend *in addition to* the base categories (`Person`, `Organization`, `Location`,
/// `Email`, `Phone`). No weights are loaded and no model is reloaded — a vertical is
/// arguments to an inference call, nothing more. See
/// `superpowers/specs/2026-07-31-vertical-model-specialisation-design.md` §4.1.
///
/// # Footgun: the alias-table collision
///
/// [`crate::pii::ner::to_pii_category`] maps some `Custom` label strings onto existing
/// [`crate::pii::PiiCategory`] variants *before* they would otherwise become
/// `PiiCategory::Custom(label)` — for example the label `"iban"` (case-insensitively)
/// becomes [`crate::pii::PiiCategory::Iban`], not `Custom("iban")`. A vertical author
/// who picks a label that happens to collide with that alias table gets the aliased
/// category's downstream behaviour (its pseudonym token name, its place in exports,
/// its interaction with other detectors) instead of a bespoke `Custom` category. This
/// is deliberate for the *known* aliases (a vertical calling something `"iban"` should
/// get real IBAN handling) but is easy to trip over by accident with a near-miss label
/// (`"iban_number"` does *not* alias and stays `Custom("iban_number")`). Check
/// `to_pii_category`'s alias table before choosing a label if the distinction matters
/// to you.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct VerticalConfig {
    /// Stable identifier recorded in the audit chain.
    pub id: String,
    /// Zero-shot labels handed to the NER backend in addition to the base categories.
    pub labels: Vec<String>,
}

/// Zero-shot labels for [`VerticalConfig::comprehensive`], one per [`crate::pii::PiiCategory`]
/// variant that is not already one of [`BASE_CATEGORY_NAMES`] (`Email`, `PhoneNumber`,
/// `Person`, `Organization` are excluded: the base categories already ask the model for
/// these concepts directly, via `EntityCategory::Email`/`Phone`/`Person`/`Organization`
/// rather than a `Custom` label, so requesting them again under a second label spelling
/// would just ask twice for the same thing). Labels that collide with
/// [`crate::pii::ner::to_pii_category`]'s alias table (`address`, `ssn`, `passport`,
/// `credit_card`, `iban`, `full_name`) use that table's exact spelling so detections land
/// on their real [`crate::pii::PiiCategory`] variant instead of falling through to
/// `Custom(label)`.
const COMPREHENSIVE_LABELS: [&str; 41] = [
    "address",
    "ssn",
    "passport",
    "drivers_license",
    "eu_vat",
    "national_id",
    "tax_id",
    "credit_card",
    "iban",
    "bank_account",
    "routing_number",
    "swift_bic",
    "crypto_wallet",
    "medical_record_number",
    "health_plan_number",
    "diagnosis",
    "medication",
    "username",
    "password",
    "api_key",
    "secret_token",
    "jwt_token",
    "ip_address",
    "mac_address",
    "url",
    "license_plate",
    "vehicle_vin",
    "date_of_birth",
    "full_name",
    "first_name",
    "middle_name",
    "last_name",
    "street_address",
    "city",
    "state_or_region",
    "postal_code",
    "country",
    "government_id",
    "payment_card",
    "card_expiry",
    "card_cvv",
];

impl VerticalConfig {
    /// A built-in vertical requesting every [`crate::pii::PiiCategory`] the taxonomy
    /// defines, beyond the five categories every pipeline already requests by default.
    /// Strictly opt-in: nothing constructs this except a caller that asks for it by
    /// name (e.g. `--vertical comprehensive` on the CLI) — [`PipelineConfig::default`]
    /// still leaves `vertical` at `None`. See [`COMPREHENSIVE_LABELS`] for what's
    /// excluded and why.
    pub fn comprehensive() -> Self {
        Self {
            id: "comprehensive".to_string(),
            labels: COMPREHENSIVE_LABELS.iter().map(|s| s.to_string()).collect(),
        }
    }

    /// Validate this vertical's shape.
    ///
    /// Checks, independent of redaction mode and of whether a model is actually
    /// loaded: `id` is non-empty, has no leading/trailing whitespace, and does not
    /// contain `@` (reserved as [`Self::provenance_id`]'s separator); `labels` is
    /// non-empty; every label is non-empty after trimming and has no leading/trailing
    /// whitespace of its own (an untrimmed label would otherwise pass this check but
    /// flow untrimmed into detection and `provenance_id`'s hash input); no label
    /// contains `[`, `:`, or `]` (mirroring
    /// [`crate::redaction::pseudonym::category_label`], since a vertical label that
    /// cannot be pseudonymised is a configuration error regardless of the configured
    /// redaction mode); no label case-insensitively duplicates a base NER category
    /// name (`person`, `organization`, `location`, `email`, `phone` — already
    /// requested by default, see [`crate::pii::ner::DEFAULT_CATEGORIES`]); and no two
    /// labels are equal after case-folding.
    ///
    /// # Errors
    ///
    /// Returns [`PiiError::InvalidVertical`] naming this vertical's `id` and the first
    /// reason it failed.
    pub fn validate(&self) -> Result<(), PiiError> {
        let fail = |reason: String| {
            Err(PiiError::InvalidVertical {
                id: self.id.clone(),
                reason,
            })
        };

        if self.id.trim().is_empty() {
            return fail("id must not be empty".to_string());
        }
        if self.id.trim() != self.id {
            return fail("id must not have leading or trailing whitespace".to_string());
        }
        if self.id.contains('@') {
            return fail(
                "id must not contain '@' (reserved as the provenance id separator)".to_string(),
            );
        }
        if self.labels.is_empty() {
            return fail("labels must not be empty".to_string());
        }

        let mut seen = HashSet::new();
        for label in &self.labels {
            let trimmed = label.trim();
            if trimmed.is_empty() {
                return fail(format!("label {label:?} is empty after trimming"));
            }
            if trimmed.contains('[') || trimmed.contains(':') || trimmed.contains(']') {
                return fail(format!(
                    "label {trimmed:?} contains a token delimiter ('[', ':' or ']')"
                ));
            }
            if trimmed != label {
                return fail(format!(
                    "label {label:?} has leading or trailing whitespace; trim it before configuring"
                ));
            }
            if BASE_CATEGORY_NAMES.contains(&trimmed.to_ascii_lowercase().as_str()) {
                return fail(format!(
                    "label {trimmed:?} duplicates a base NER category name ({BASE_CATEGORY_NAMES:?}); \
                     it is already requested by default and does not need a vertical label"
                ));
            }
            if !seen.insert(trimmed.to_ascii_lowercase()) {
                return fail(format!(
                    "label {trimmed:?} duplicates another label (case-insensitively)"
                ));
            }
        }

        Ok(())
    }

    /// The value recorded in the audit chain for detections this vertical produced.
    ///
    /// `"<id>@<first 8 hex of blake3 over the sorted, case-folded label set>"`, e.g.
    /// `finance@3f9a1c02`. An id alone would be a false provenance claim: the same id
    /// with a different label set detects different things, and the audit record's job
    /// is to say what *was* detectable at the time. Sorting and case-folding the labels
    /// before hashing makes the digest stable under reordering or a stray case change
    /// in config, while still changing whenever the actual label *set* changes — see
    /// [`AuditEntry::vertical`](crate::audit::AuditEntry::vertical).
    pub fn provenance_id(&self) -> String {
        let mut labels: Vec<String> = self.labels.iter().map(|l| l.to_ascii_lowercase()).collect();
        labels.sort();
        let mut hasher = blake3::Hasher::new();
        for label in &labels {
            hasher.update(label.as_bytes());
            // A separator so ["ab", "c"] and ["a", "bc"] hash differently.
            hasher.update(b"\0");
        }
        let digest = hasher.finalize().to_hex();
        format!("{}@{}", self.id, &digest[..PROVENANCE_DIGEST_LEN])
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
        assert!(
            config.vertical.is_none(),
            "no vertical is active until an operator opts in"
        );
    }

    fn finance_vertical() -> VerticalConfig {
        VerticalConfig {
            id: "finance".to_string(),
            labels: vec!["swift_code".to_string(), "account_number".to_string()],
        }
    }

    #[test]
    fn should_accept_a_well_formed_vertical() {
        finance_vertical().validate().unwrap();
    }

    #[test]
    fn should_reject_a_vertical_with_an_empty_id() {
        let vertical = VerticalConfig {
            id: String::new(),
            ..finance_vertical()
        };
        let error = vertical.validate().unwrap_err();
        assert!(matches!(error, PiiError::InvalidVertical { .. }));
    }

    #[test]
    fn should_reject_a_vertical_with_no_labels() {
        let vertical = VerticalConfig {
            labels: Vec::new(),
            ..finance_vertical()
        };
        assert!(matches!(
            vertical.validate(),
            Err(PiiError::InvalidVertical { .. })
        ));
    }

    #[test]
    fn should_reject_a_vertical_with_a_blank_label() {
        let vertical = VerticalConfig {
            labels: vec!["   ".to_string()],
            ..finance_vertical()
        };
        assert!(matches!(
            vertical.validate(),
            Err(PiiError::InvalidVertical { .. })
        ));
    }

    #[test]
    fn should_reject_a_vertical_label_containing_a_token_delimiter() {
        for bad in ["swift[code", "swift:code", "swift]code"] {
            let vertical = VerticalConfig {
                labels: vec![bad.to_string()],
                ..finance_vertical()
            };
            let error = vertical.validate().unwrap_err();
            assert!(
                matches!(error, PiiError::InvalidVertical { .. }),
                "label {bad:?} must be rejected"
            );
        }
    }

    #[test]
    fn should_reject_a_vertical_label_with_leading_or_trailing_whitespace() {
        let vertical = VerticalConfig {
            labels: vec![" swift_code ".to_string()],
            ..finance_vertical()
        };
        assert!(matches!(
            vertical.validate(),
            Err(PiiError::InvalidVertical { .. })
        ));
    }

    #[test]
    fn should_reject_a_vertical_label_that_duplicates_a_base_category_name() {
        for base in ["person", "Organization", "LOCATION", "email", "Phone"] {
            let vertical = VerticalConfig {
                labels: vec![base.to_string()],
                ..finance_vertical()
            };
            let error = vertical.validate().unwrap_err();
            assert!(
                matches!(error, PiiError::InvalidVertical { .. }),
                "label {base:?} must be rejected"
            );
        }
    }

    #[test]
    fn should_reject_a_vertical_id_with_leading_or_trailing_whitespace() {
        let vertical = VerticalConfig {
            id: " finance ".to_string(),
            ..finance_vertical()
        };
        assert!(matches!(
            vertical.validate(),
            Err(PiiError::InvalidVertical { .. })
        ));
    }

    #[test]
    fn should_reject_a_vertical_id_containing_at_sign() {
        let vertical = VerticalConfig {
            id: "finance@prod".to_string(),
            ..finance_vertical()
        };
        assert!(matches!(
            vertical.validate(),
            Err(PiiError::InvalidVertical { .. })
        ));
    }

    #[test]
    fn should_reject_duplicate_labels_after_case_folding() {
        let vertical = VerticalConfig {
            labels: vec!["Swift_Code".to_string(), "swift_code".to_string()],
            ..finance_vertical()
        };
        assert!(matches!(
            vertical.validate(),
            Err(PiiError::InvalidVertical { .. })
        ));
    }

    #[test]
    fn should_produce_a_stable_provenance_id_regardless_of_label_order() {
        let a = VerticalConfig {
            id: "finance".to_string(),
            labels: vec!["swift_code".to_string(), "account_number".to_string()],
        };
        let b = VerticalConfig {
            id: "finance".to_string(),
            labels: vec!["account_number".to_string(), "swift_code".to_string()],
        };
        assert_eq!(a.provenance_id(), b.provenance_id());
    }

    #[test]
    fn should_produce_a_stable_provenance_id_regardless_of_label_case() {
        let a = VerticalConfig {
            id: "finance".to_string(),
            labels: vec!["Swift_Code".to_string(), "Account_Number".to_string()],
        };
        let b = VerticalConfig {
            id: "finance".to_string(),
            labels: vec!["swift_code".to_string(), "account_number".to_string()],
        };
        assert_eq!(a.provenance_id(), b.provenance_id());
    }

    #[test]
    fn should_change_the_provenance_id_when_a_label_is_added() {
        let base = finance_vertical();
        let mut extended = base.clone();
        extended.labels.push("routing_number".to_string());
        assert_ne!(base.provenance_id(), extended.provenance_id());
    }

    #[test]
    fn should_prefix_the_provenance_id_with_the_vertical_id() {
        let vertical = finance_vertical();
        let provenance = vertical.provenance_id();
        assert!(
            provenance.starts_with("finance@"),
            "expected a `finance@<digest>` provenance id, got {provenance:?}"
        );
        let digest = provenance.strip_prefix("finance@").unwrap();
        assert_eq!(
            digest.len(),
            PROVENANCE_DIGEST_LEN,
            "digest must be the first PROVENANCE_DIGEST_LEN hex chars"
        );
        assert!(digest.chars().all(|c| c.is_ascii_hexdigit()));
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
    fn should_accept_the_comprehensive_vertical() {
        VerticalConfig::comprehensive().validate().unwrap();
    }

    #[test]
    fn should_size_the_comprehensive_vertical_to_the_taxonomy_minus_the_base_categories() {
        // PiiCategory has 45 named variants (33 + 12 hybrid) plus `Custom(String)`; 4 of the named
        // variants (Email, PhoneNumber, Person, Organization) are already requested
        // via the base categories, so the comprehensive vertical adds the remaining 41.
        let comprehensive = VerticalConfig::comprehensive();
        assert_eq!(comprehensive.labels.len(), 41);
        assert_eq!(comprehensive.id, "comprehensive");
    }

    #[test]
    fn should_report_the_path_when_a_config_file_is_missing() {
        let error = PipelineConfig::from_file(Path::new("/nonexistent/hacienda.toml")).unwrap_err();
        assert!(matches!(error, PiiError::ConfigIo { .. }));
        assert!(error.to_string().contains("/nonexistent/hacienda.toml"));
    }

    #[test]
    fn should_apply_per_category_threshold_offsets() {
        let mut config = PipelineConfig {
            model_threshold_default: 0.5,
            ..Default::default()
        };
        // Person family gets boosted
        assert_eq!(config.effective_threshold(&PiiCategory::Person), 0.65);
        assert_eq!(config.effective_threshold(&PiiCategory::FirstName), 0.65);
        assert_eq!(config.effective_threshold(&PiiCategory::LastName), 0.65);
        // High-precision categories get lowered
        assert_eq!(config.effective_threshold(&PiiCategory::Email), 0.48);
        assert_eq!(config.effective_threshold(&PiiCategory::CreditCard), 0.48);
        // Override via map
        config.model_thresholds.insert(PiiCategory::Person, 0.7);
        assert_eq!(config.effective_threshold(&PiiCategory::Person), 0.7);
        // Default fallback
        assert_eq!(config.effective_threshold(&PiiCategory::Ssn), 0.5);
    }
}
