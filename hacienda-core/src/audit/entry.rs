//! Audit entries and the chain-hash function that binds them together.

use chrono::Utc;
use serde::{Deserialize, Serialize};

/// The redaction that was applied to a span, as recorded in the audit log.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RedactionAction {
    Mask,
    Hash,
    Pseudonymize,
    Remove,
    Custom(String),
}

// There is deliberately no `From<RedactionMode> for RedactionAction`.
//
// The conversion cannot be total: `Custom` carries the template that was applied, and a
// mode does not determine one. The impl that used to live here filled the gap with the
// literal string "template", which made every Custom redaction in the chain identical and
// so answered none of the questions the field exists to answer.
//
// `RedactionEngine::audit_action` performs the conversion where the template is in scope.

/// Which detector produced the entity the entry describes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EntitySource {
    Regex,
    Model,
}

impl std::fmt::Display for EntitySource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EntitySource::Regex => write!(f, "regex"),
            EntitySource::Model => write!(f, "model"),
        }
    }
}

impl From<crate::pii::types::EntitySource> for EntitySource {
    fn from(source: crate::pii::types::EntitySource) -> Self {
        match source {
            crate::pii::types::EntitySource::Regex => Self::Regex,
            crate::pii::types::EntitySource::Model => Self::Model,
        }
    }
}

/// A single tamper-evident record of one PII redaction.
///
/// The original span is never stored — only its blake3 digest.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditEntry {
    pub id: String,
    pub timestamp: String,
    pub category: String,
    pub action: RedactionAction,
    pub span_hash: String,
    pub span_length: u32,
    pub confidence: Option<f32>,
    pub source: EntitySource,
    pub pipeline_version: String,
    pub config_hash: String,
    /// blake3 over the previous chain hash and this entry's identifying fields.
    pub chain_hash: String,
}

/// Everything needed to mint an [`AuditEntry`] except its position in the chain.
#[derive(Debug, Clone)]
pub struct AuditEntryInput {
    pub id: String,
    pub category: String,
    pub action: RedactionAction,
    pub span_hash: String,
    pub span_length: u32,
    pub confidence: Option<f32>,
    pub source: EntitySource,
    pub pipeline_version: String,
    pub config_hash: String,
}

impl AuditEntry {
    /// Mint an entry that extends the chain ending at `prev_chain_hash`.
    ///
    /// `seq` is the 0-based position this entry will occupy, and must match what
    /// [`AuditChain::append`](crate::audit::AuditChain::append) expects.
    pub fn new(input: AuditEntryInput, prev_chain_hash: &str, seq: u64) -> Self {
        let chain_hash = compute_chain_hash(
            prev_chain_hash,
            seq,
            &input.id,
            &input.category,
            &input.action,
            &input.span_hash,
            &input.config_hash,
        );

        Self {
            id: input.id,
            timestamp: Utc::now().to_rfc3339(),
            category: input.category,
            action: input.action,
            span_hash: input.span_hash,
            span_length: input.span_length,
            confidence: input.confidence,
            source: input.source,
            pipeline_version: input.pipeline_version,
            config_hash: input.config_hash,
            chain_hash,
        }
    }
}

/// Compute the chain hash linking an entry to its predecessor.
///
/// The timestamp is deliberately excluded so verification is reproducible.
pub fn compute_chain_hash(
    prev_chain_hash: &str,
    seq: u64,
    id: &str,
    category: &str,
    action: &RedactionAction,
    span_hash: &str,
    config_hash: &str,
) -> String {
    let mut hasher = blake3::Hasher::new();
    hasher.update(prev_chain_hash.as_bytes());
    hasher.update(&seq.to_le_bytes());
    hasher.update(id.as_bytes());
    hasher.update(category.as_bytes());
    let action_str = serde_json::to_string(action).unwrap_or_default();
    hasher.update(action_str.as_bytes());
    hasher.update(span_hash.as_bytes());
    hasher.update(config_hash.as_bytes());
    hasher.finalize().to_hex().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(id: &str) -> AuditEntryInput {
        AuditEntryInput {
            id: id.into(),
            category: "Email".into(),
            action: RedactionAction::Mask,
            span_hash: "abc".into(),
            span_length: 10,
            confidence: Some(1.0),
            source: EntitySource::Regex,
            pipeline_version: "1.0".into(),
            config_hash: "cfg".into(),
        }
    }

    #[test]
    fn should_produce_the_same_chain_hash_for_the_same_inputs() {
        let a = AuditEntry::new(input("id-1"), "prev", 0);
        let b = AuditEntry::new(input("id-1"), "prev", 0);
        assert_eq!(a.chain_hash, b.chain_hash);
    }

    #[test]
    fn should_produce_a_different_chain_hash_for_a_different_sequence_number() {
        let a = AuditEntry::new(input("id-1"), "prev", 0);
        let b = AuditEntry::new(input("id-1"), "prev", 1);
        assert_ne!(a.chain_hash, b.chain_hash);
    }

    #[test]
    fn should_produce_a_different_chain_hash_for_a_different_predecessor() {
        let a = AuditEntry::new(input("id-1"), "prev-a", 0);
        let b = AuditEntry::new(input("id-1"), "prev-b", 0);
        assert_ne!(a.chain_hash, b.chain_hash);
    }

    // The mode-to-action mapping moved to `RedactionEngine::audit_action` along with the
    // `From` impl it used to test; coverage lives in
    // `redaction::engine::tests::should_record_the_applied_action_for_every_mode`.
}
