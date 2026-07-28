//! Redaction configuration and result types.

use serde::{Deserialize, Serialize};

/// How a detected span is rewritten in the output text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum RedactionMode {
    /// Replace with the category's redaction template, e.g. `[EMAIL]`.
    Mask,
    /// Replace with a short blake3 digest of the original span.
    Hash,
    /// Replace with a category placeholder that keeps the span recognisable, e.g. `[EMAIL:****]`.
    #[default]
    Pseudonymize,
    /// Delete the span entirely.
    Remove,
    /// Replace using [`RedactionConfig::custom_template`].
    Custom,
}

impl std::str::FromStr for RedactionMode {
    type Err = crate::redaction::RedactionError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_ascii_lowercase().as_str() {
            "mask" => Ok(Self::Mask),
            "hash" => Ok(Self::Hash),
            "pseudonymize" => Ok(Self::Pseudonymize),
            "remove" => Ok(Self::Remove),
            "custom" => Ok(Self::Custom),
            other => Err(crate::redaction::RedactionError::UnknownMode(
                other.to_string(),
            )),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RedactionConfig {
    pub mode: RedactionMode,
    /// Template used when `mode` is [`RedactionMode::Custom`].
    ///
    /// `{{ENTITY}}` expands to the category name and `{{TEXT}}` to the original span.
    pub custom_template: Option<String>,
    /// Keep the shape of the original span (length, separators) where the mode allows it.
    pub preserve_format: bool,
}

impl Default for RedactionConfig {
    fn default() -> Self {
        Self {
            mode: RedactionMode::Pseudonymize,
            custom_template: None,
            preserve_format: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RedactionResult {
    pub text: String,
    pub audit_log: Vec<RedactionAuditEntry>,
    pub metrics: RedactionMetrics,
}

/// One hash-chained record of a single redacted span.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RedactionAuditEntry {
    pub category: String,
    pub action: RedactionMode,
    /// Which detector produced the span. Needed to attribute a redaction in the
    /// persistent audit chain without re-running detection.
    pub source: crate::pii::types::EntitySource,
    /// blake3 digest of the original span. The span itself is never recorded.
    pub span_hash: String,
    pub span_length: u32,
    pub confidence: Option<f32>,
    pub timestamp: u64,
    pub chain_hash: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RedactionMetrics {
    pub redaction_ms: u64,
    pub entities_detected: u32,
    pub entities_redacted: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn should_parse_every_known_mode_name() {
        assert_eq!(
            "mask".parse::<RedactionMode>().unwrap(),
            RedactionMode::Mask
        );
        assert_eq!(
            "HASH".parse::<RedactionMode>().unwrap(),
            RedactionMode::Hash
        );
        assert_eq!(
            "pseudonymize".parse::<RedactionMode>().unwrap(),
            RedactionMode::Pseudonymize
        );
        assert_eq!(
            "remove".parse::<RedactionMode>().unwrap(),
            RedactionMode::Remove
        );
        assert_eq!(
            "custom".parse::<RedactionMode>().unwrap(),
            RedactionMode::Custom
        );
    }

    #[test]
    fn should_reject_an_unknown_mode_name() {
        assert!("shred".parse::<RedactionMode>().is_err());
    }

    #[test]
    fn should_default_to_pseudonymize() {
        assert_eq!(RedactionConfig::default().mode, RedactionMode::Pseudonymize);
    }
}
