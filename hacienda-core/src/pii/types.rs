//! Entity and pattern types shared by the regex engine, the model backend, and the merger.

use serde::{Deserialize, Serialize};
use std::fmt;

/// Category of personally identifiable information a detected span belongs to.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PiiCategory {
    Email,
    PhoneNumber,
    Address,
    Ssn,
    PassportNumber,
    DriversLicense,
    NationalId,
    TaxId,
    CreditCard,
    Iban,
    BankAccount,
    RoutingNumber,
    SwiftBic,
    CryptoWallet,
    MedicalRecordNumber,
    HealthPlanNumber,
    Diagnosis,
    Medication,
    Username,
    Password,
    ApiKey,
    SecretToken,
    JwtToken,
    IpAddress,
    MacAddress,
    Url,
    LicensePlate,
    VehicleVin,
    DateOfBirth,
    FullName,
    Person,
    Organization,
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
pub struct RegexEntity {
    pub category: PiiCategory,
    /// Byte offset of the first byte of the span.
    pub start: u32,
    /// Byte offset one past the last byte of the span.
    pub end: u32,
    pub confidence: f32,
    pub format_preserving: bool,
    pub redact_template: String,
}

impl RegexEntity {
    pub fn new(category: PiiCategory, start: u32, end: u32) -> Self {
        let redact_template = format!("[{category:?}]").to_uppercase();
        Self {
            category,
            start,
            end,
            confidence: 1.0,
            format_preserving: false,
            redact_template,
        }
    }
}

/// A single built-in detection pattern and how its matches should be redacted.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PatternMeta {
    pub category: PiiCategory,
    pub pattern: String,
    pub format_preserving: bool,
    pub redact_template: String,
}

/// A span produced by a statistical (NER model) backend.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelEntity {
    pub category: PiiCategory,
    pub text: String,
    pub start: u32,
    pub end: u32,
    pub confidence: f32,
}

/// Which detector produced a merged entity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EntitySource {
    Regex,
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
pub enum MergePriority {
    RegexFirst,
    HigherConfidence,
    LongerSpan,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MergeConfig {
    /// Overlap ratio above which two spans are considered the same detection.
    pub overlap_threshold: f32,
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
