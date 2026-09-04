//! Entity and pattern types shared by the regex engine, the model backend, and the merger.

use serde::{Deserialize, Serialize};
use std::fmt;

/// Category of personally identifiable information a detected span belongs to.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
/// PiiCategory enum
pub enum PiiCategory {
    /// Email variant
    Email,
    /// PhoneNumber variant
    PhoneNumber,
    /// Address variant
    Address,
    /// Ssn variant
    Ssn,
    /// PassportNumber variant
    PassportNumber,
    /// DriversLicense variant
    DriversLicense,
    /// EU intra-community VAT number: a 2-letter member-state code (`EL` for Greece,
    /// not `GR`) followed by up to 12 digits. Added for Track C2 — the French client
    /// base's most common gap between the browser's 9 regexes and the Rust set.
    EuVat,
    /// NationalId variant
    NationalId,
    /// TaxId variant
    TaxId,
    /// CreditCard variant
    CreditCard,
    /// Iban variant
    Iban,
    /// BankAccount variant
    BankAccount,
    /// RoutingNumber variant
    RoutingNumber,
    /// SwiftBic variant
    SwiftBic,
    /// CryptoWallet variant
    CryptoWallet,
    /// MedicalRecordNumber variant
    MedicalRecordNumber,
    /// HealthPlanNumber variant
    HealthPlanNumber,
    /// Diagnosis variant
    Diagnosis,
    /// Medication variant
    Medication,
    /// Username variant
    Username,
    /// Password variant
    Password,
    /// ApiKey variant
    ApiKey,
    /// SecretToken variant
    SecretToken,
    /// JwtToken variant
    JwtToken,
    /// IpAddress variant
    IpAddress,
    /// MacAddress variant
    MacAddress,
    /// Url variant
    Url,
    /// LicensePlate variant
    LicensePlate,
    /// VehicleVin variant
    VehicleVin,
    /// DateOfBirth variant
    DateOfBirth,
    /// FullName variant
    FullName,
    /// Person variant
    Person,
    /// Organization variant
    Organization,
    /// FirstName variant
    FirstName,
    /// MiddleName variant
    MiddleName,
    /// LastName variant
    LastName,
    /// StreetAddress variant
    StreetAddress,
    /// City variant
    City,
    /// StateOrRegion variant
    StateOrRegion,
    /// PostalCode variant
    PostalCode,
    /// Country variant
    Country,
    /// GovernmentId variant
    GovernmentId,
    /// PaymentCard variant
    PaymentCard,
    /// CardExpiry variant
    CardExpiry,
    /// CardCvv variant
    CardCvv,
    /// Custom variant
    Custom(String),
}

impl fmt::Display for PiiCategory {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PiiCategory::Custom(s) => write!(f, "{s}"),
            other => write!(f, "{other:?}"),
        }
    }
}

/// A span matched by the deterministic regex engine.
#[derive(Debug, Clone, Serialize, Deserialize)]
/// RegexEntity struct
pub struct RegexEntity {
    /// category field
    pub category: PiiCategory,
    /// Byte offset of the first byte of the span.
    pub start: u32,
    /// Byte offset one past the last byte of the span.
    pub end: u32,
    /// confidence field
    pub confidence: f32,
    /// format_preserving field
    pub format_preserving: bool,
    /// redact_template field
    pub redact_template: String,
    /// Copied from the originating [`PatternMeta::context_words`] at match time, so
    /// [`crate::pii::context::enhance`] can look up this span's context words without
    /// needing the pattern set that produced it back in scope. `#[serde(skip)]`: a
    /// `&'static` slice cannot round-trip through serde, and (as with
    /// [`PatternMeta::validator`]) nothing actually (de)serializes `RegexEntity` today.
    #[serde(skip)]
    /// context_words field
    pub context_words: &'static [&'static str],
}

impl RegexEntity {
/// new function
    pub fn new(category: PiiCategory, start: u32, end: u32) -> Self {
        let redact_template = format!("[{category:?}]").to_uppercase();
        Self {
            category,
            start,
            end,
            confidence: 1.0,
            format_preserving: false,
            redact_template,
            context_words: &[],
        }
    }
}

/// A single built-in detection pattern and how its matches should be redacted.
#[derive(Debug, Clone, Serialize, Deserialize)]
/// PatternMeta struct
pub struct PatternMeta {
    /// category field
    pub category: PiiCategory,
    /// pattern field
    pub pattern: String,
    /// format_preserving field
    pub format_preserving: bool,
    /// redact_template field
    pub redact_template: String,
    /// Confidence a match gets when [`validator`](Self::validator) is absent, or returns
    /// `None` (no opinion). Defaults to `1.0` via [`PatternMeta::new`] so every pre-existing
    /// pattern keeps today's behavior; only patterns with a real checksum
    /// ([`crate::pii::validators`]) set this lower, since an unvalidated match in those
    /// categories is genuinely less certain than a plain regex hit.
    #[serde(default = "default_base_confidence")]
    /// base_confidence field
    pub base_confidence: f32,
    /// Checksum/structural validator run against the matched text
    /// ([`crate::pii::validators`]'s `Option<bool>` contract: `Some(true)` promotes
    /// confidence to `1.0`, `Some(false)` discards the match, `None` keeps
    /// `base_confidence`). `#[serde(skip)]`: a function pointer cannot round-trip through
    /// serde, and nothing in this codebase actually (de)serializes `PatternMeta` today —
    /// this only guards against a future config-file/API surface silently losing it.
    #[serde(skip)]
    /// validator field
    pub validator: Option<fn(&str) -> Option<bool>>,
    /// Words that, found near a match, boost its confidence toward `1.0`
    /// ([`crate::pii::context`]). Empty for patterns with no calibrated context word list.
    #[serde(skip)]
    /// context_words field
    pub context_words: &'static [&'static str],
}

fn default_base_confidence() -> f32 {
    1.0
}

impl PatternMeta {
    /// A plain pattern with no validator or context words, at full confidence — the shape
    /// every pattern had before checksum validation and context boosting existed.
    /// [`builtin_patterns`](super::patterns::builtin_patterns) constructs every pattern
    /// through this (or the `with_*` builders below) rather than a struct literal, so
    /// adding a field here never breaks an existing pattern definition.
    pub fn new(
        category: PiiCategory,
        pattern: impl Into<String>,
        format_preserving: bool,
        redact_template: impl Into<String>,
    ) -> Self {
        Self {
            category,
            pattern: pattern.into(),
            format_preserving,
            redact_template: redact_template.into(),
            base_confidence: 1.0,
            validator: None,
            context_words: &[],
        }
    }

    /// Starting confidence for a match this pattern produces, before
    /// [`validator`](Self::validator) or [`crate::pii::context`] boosting run. Only
    /// patterns with a real checksum should lower this — see the field's own doc.
    pub fn with_base_confidence(mut self, base_confidence: f32) -> Self {
        self.base_confidence = base_confidence;
        self
    }

    /// Attach a checksum/structural validator — see the field's own doc for the
    /// `Option<bool>` contract.
    pub fn with_validator(mut self, validator: fn(&str) -> Option<bool>) -> Self {
        self.validator = Some(validator);
        self
    }

    /// Attach the context words that boost this category's confidence when found near a
    /// match — see [`crate::pii::context`].
    pub fn with_context(mut self, context_words: &'static [&'static str]) -> Self {
        self.context_words = context_words;
        self
    }
}

/// A span produced by a statistical (NER model) backend.
#[derive(Debug, Clone, Serialize, Deserialize)]
/// ModelEntity struct
pub struct ModelEntity {
    /// category field
    pub category: PiiCategory,
    /// text field
    pub text: String,
    /// start field
    pub start: u32,
    /// end field
    pub end: u32,
    /// confidence field
    pub confidence: f32,
}

/// Which detector produced a merged entity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
/// EntitySource enum
pub enum EntitySource {
    /// Regex variant
    Regex,
    /// Model variant
    Model,
}

impl fmt::Display for EntitySource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            EntitySource::Regex => write!(f, "regex"),
            EntitySource::Model => write!(f, "model"),
        }
    }
}

/// Tie-break rule applied when two detections overlap.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
/// MergePriority enum
pub enum MergePriority {
    /// RegexFirst variant
    RegexFirst,
    /// HigherConfidence variant
    HigherConfidence,
    /// LongerSpan variant
    LongerSpan,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
/// MergeConfig struct
pub struct MergeConfig {
    /// Overlap ratio above which two spans are considered the same detection.
    pub overlap_threshold: f32,
    /// priority field
    pub priority: MergePriority,
    /// Confidence difference required before a candidate displaces an existing span.
    pub confidence_epsilon: f32,
}

impl Default for MergeConfig {
    fn default() -> Self {
        Self {
            overlap_threshold: 0.5,
            priority: MergePriority::RegexFirst,
            confidence_epsilon: 0.05,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn should_display_builtin_category_as_variant_name() {
        assert_eq!(PiiCategory::Email.to_string(), "Email");
        assert_eq!(PiiCategory::CreditCard.to_string(), "CreditCard");
    }

    #[test]
    fn should_display_custom_category_as_its_label() {
        assert_eq!(
            PiiCategory::Custom("EmployeeId".into()).to_string(),
            "EmployeeId"
        );
    }

    #[test]
    fn should_build_uppercase_redact_template_from_category() {
        let entity = RegexEntity::new(PiiCategory::Email, 0, 5);
        assert_eq!(entity.redact_template, "[EMAIL]");
        assert_eq!(entity.confidence, 1.0);
    }
}
