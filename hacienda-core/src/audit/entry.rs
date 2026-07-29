//! Audit entries and the chain-hash function that binds them together.

use chrono::Utc;
use serde::{Deserialize, Serialize};

/// The redaction that was applied to a span, as recorded in the audit log.
///
/// `Reveal` is included because an audit chain that omits "who accessed the
/// unredacted span text" is not credible for a compliance product — see §7 of
/// the design spec. The `span_hash` field on the entry carries the digest of
/// the revealed text so the record is non-repudiable without storing the
/// plaintext.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RedactionAction {
    Mask,
    Hash,
    Pseudonymize,
    Remove,
    /// Span text was returned to an authorised caller in plaintext form.
    ///
    /// Only appended when `SpanText::Include` is used with a `Caller::Principal`
    /// that holds `Capability::PiiReveal`. The `span_hash` field on the entry
    /// carries the blake3 digest of the revealed text.
    Reveal,
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
    /// The authenticated principal this entry is attributable to, or `None` for an
    /// in-process caller ([`Caller::Trusted`](crate::auth::Caller::Trusted)).
    ///
    /// Covered by [`compute_chain_hash`], so attribution cannot be rewritten after the
    /// fact without breaking verification. `None` hashes as the empty string — which is
    /// exactly what entries written before this field existed hashed as, so older chains
    /// still verify.
    #[serde(default)]
    pub principal: Option<String>,
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
    /// See [`AuditEntry::principal`].
    pub principal: Option<String>,
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
            ChainHashFields {
                id: &input.id,
                category: &input.category,
                action: &input.action,
                span_hash: &input.span_hash,
                config_hash: &input.config_hash,
                principal: input.principal.as_deref(),
            },
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
            principal: input.principal,
            chain_hash,
        }
    }

    /// The fields of this entry that [`compute_chain_hash`] covers.
    pub fn chain_hash_fields(&self) -> ChainHashFields<'_> {
        ChainHashFields {
            id: &self.id,
            category: &self.category,
            action: &self.action,
            span_hash: &self.span_hash,
            config_hash: &self.config_hash,
            principal: self.principal.as_deref(),
        }
    }
}

/// The entry fields that [`compute_chain_hash`] covers.
///
/// A named struct rather than positional arguments because this field list *is* the
/// definition of what the chain protects. Adding a field here is a deliberate, reviewable
/// act, and no caller can silently transpose two `&str` arguments — which for a
/// tamper-evidence primitive would be a defect that only shows up as a verification
/// failure months later.
#[derive(Debug, Clone, Copy)]
pub struct ChainHashFields<'a> {
    pub id: &'a str,
    pub category: &'a str,
    pub action: &'a RedactionAction,
    pub span_hash: &'a str,
    pub config_hash: &'a str,
    pub principal: Option<&'a str>,
}

/// Compute the chain hash linking an entry to its predecessor.
///
/// The timestamp is deliberately excluded so verification is reproducible.
pub fn compute_chain_hash(prev_chain_hash: &str, seq: u64, fields: ChainHashFields<'_>) -> String {
    let mut hasher = blake3::Hasher::new();
    hasher.update(prev_chain_hash.as_bytes());
    hasher.update(&seq.to_le_bytes());
    hasher.update(fields.id.as_bytes());
    hasher.update(fields.category.as_bytes());
    let action_str = serde_json::to_string(fields.action).unwrap_or_default();
    hasher.update(action_str.as_bytes());
    hasher.update(fields.span_hash.as_bytes());
    hasher.update(fields.config_hash.as_bytes());
    // Absent attribution hashes as the empty string so that chains written before this
    // field existed continue to verify byte-for-byte.
    hasher.update(fields.principal.unwrap_or("").as_bytes());
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
            principal: None,
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

    /// Attribution is inside the hash, so re-pointing an entry at a different principal
    /// invalidates it. Without this the audit trail would answer "who" with a field an
    /// operator could edit at rest.
    #[test]
    fn should_produce_a_different_chain_hash_for_a_different_principal() {
        let anonymous = AuditEntry::new(input("id-1"), "prev", 0);
        let attributed = AuditEntry::new(
            AuditEntryInput {
                principal: Some("avocat-7".into()),
                ..input("id-1")
            },
            "prev",
            0,
        );
        assert_ne!(anonymous.chain_hash, attributed.chain_hash);
    }

    /// Absent attribution must hash as nothing at all, not as a sentinel string, so
    /// chains written before the field existed still verify.
    #[test]
    fn should_hash_an_absent_principal_as_no_bytes() {
        let fields = ChainHashFields {
            id: "id-1",
            category: "Email",
            action: &RedactionAction::Mask,
            span_hash: "abc",
            config_hash: "cfg",
            principal: None,
        };
        let empty_string_principal = ChainHashFields {
            principal: Some(""),
            ..fields
        };
        assert_eq!(
            compute_chain_hash("prev", 0, fields),
            compute_chain_hash("prev", 0, empty_string_principal)
        );
    }

    // The mode-to-action mapping moved to `RedactionEngine::audit_action` along with the
    // `From` impl it used to test; coverage lives in
    // `redaction::engine::tests::should_record_the_applied_action_for_every_mode`.
}
