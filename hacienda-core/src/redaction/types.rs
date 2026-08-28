//! Redaction configuration and result types.

use serde::{Deserialize, Serialize};

/// How a detected span is rewritten in the output text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
/// RedactionMode enum
pub enum RedactionMode {
    /// Replace with the category's redaction template, e.g. `[EMAIL]`.
    ///
    /// The default, because it is the only mode that is both irreversible and needs no
    /// configuration. A default of [`RedactionMode::Pseudonymize`] would mean the
    /// out-of-the-box path depends on a secret the operator has not yet set.
    #[default]
    /// Mask variant
    Mask,
    /// Replace with a short blake3 digest of the original span.
    Hash,
    /// Replace with a keyed, reversible token, e.g. `[EMAIL:k1:MZXW6YTB...]`.
    ///
    /// Equal values get equal tokens, so a reader can follow one pseudonymous person
    /// through a corpus, and a holder of the key can reverse it. Requires a key: see
    /// [`RedactionEngine::new`](crate::redaction::RedactionEngine::new).
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
#[serde(default, deny_unknown_fields)]
/// RedactionConfig struct
pub struct RedactionConfig {
    pub mode: RedactionMode,
    /// Template used when `mode` is [`RedactionMode::Custom`].
    ///
    /// `{{ENTITY}}` expands to the category name and `{{TEXT}}` to the original span.
    pub custom_template: Option<String>,
    /// Keep the shape of the original span (length, separators) where the mode allows it.
    pub preserve_format: bool,
    /// Which pseudonymisation key new tokens are minted under, when `mode` is
    /// [`RedactionMode::Pseudonymize`]. `None` means "whichever key the resolver reports
    /// as active".
    ///
    /// This is an *identifier*, never key material. The struct is serialised and printed
    /// by `config show`; putting a secret in it would put that secret in every dumped
    /// config, log line, and support bundle. Key material reaches the engine only through
    /// a [`KeyResolver`](crate::redaction::KeyResolver).
    #[serde(default)]
    pub key_id: Option<String>,
}

impl Default for RedactionConfig {
    fn default() -> Self {
        Self {
            mode: RedactionMode::default(),
            custom_template: None,
            preserve_format: true,
            key_id: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
/// RedactionResult struct
pub struct RedactionResult {
    pub text: String,
    pub audit_log: Vec<RedactionAuditEntry>,
    pub metrics: RedactionMetrics,
}

/// One hash-chained record of a single redacted span.
#[derive(Debug, Clone, Serialize, Deserialize)]
/// RedactionAuditEntry struct
pub struct RedactionAuditEntry {
    pub category: String,
    /// The rewrite that was applied — not merely the mode that was configured.
    ///
    /// [`RedactionAction::Custom`](crate::audit::RedactionAction::Custom) carries the
    /// template text, because "a custom template was used" does not answer the question
    /// an auditor is asking. Built by
    /// [`RedactionEngine`](crate::redaction::RedactionEngine), the only place the
    /// template is in scope.
    pub action: crate::audit::RedactionAction,
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
/// RedactionMetrics struct
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
    fn should_default_to_mask_rather_than_a_mode_that_needs_a_secret() {
        // Pseudonymize was the default and could not work without a key it had no way to
        // obtain, so it silently emitted a constant placeholder instead.
        assert_eq!(RedactionConfig::default().mode, RedactionMode::Mask);
    }

    #[test]
    fn should_not_carry_key_material_in_the_serialised_config() {
        // `config show` prints this struct. Only the key *id* may appear here.
        let json = serde_json::to_string(&RedactionConfig {
            mode: RedactionMode::Pseudonymize,
            key_id: Some("k1".into()),
            ..Default::default()
        })
        .unwrap();
        assert!(json.contains("\"key_id\":\"k1\""), "{json}");
        assert!(!json.to_lowercase().contains("material"), "{json}");
        assert!(!json.to_lowercase().contains("secret"), "{json}");
    }

    #[test]
    fn should_accept_a_config_written_before_key_id_existed() {
        let cfg: RedactionConfig = serde_json::from_str(
            r#"{"mode":"mask","custom_template":null,"preserve_format":true}"#,
        )
        .unwrap();
        assert_eq!(cfg.key_id, None);
    }
}
