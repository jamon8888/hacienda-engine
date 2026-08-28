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
/// RedactionAction enum
pub enum RedactionAction {
    /// Replace span with a fixed mask character.
    Mask,
    /// Replace span with a blake3 hash.
    Hash,
    /// Replace span with a pseudonym.
    Pseudonymize,
    /// Remove span entirely.
    Remove,
    /// Span text was returned to an authorised caller in plaintext form.
    ///
    /// Only appended when `SpanText::Include` is used with a `Caller::Principal`
    /// that holds `Capability::PiiReveal`. The `span_hash` field on the entry
    /// carries the blake3 digest of the revealed text.
    Reveal,
    /// Custom template applied to the span.
    Custom(String),
}

/// String form used by persistence backends (e.g. the Postgres `action` TEXT column).
///
/// Unit variants render as their `snake_case` name, matching the `Serialize` convention
/// used everywhere else this type crosses a boundary. `Custom` is prefixed so the
/// template it carries round-trips exactly, including templates containing `:` — only
/// the first `:` is treated as the delimiter on parse.
impl std::fmt::Display for RedactionAction {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RedactionAction::Mask => write!(f, "mask"),
            RedactionAction::Hash => write!(f, "hash"),
            RedactionAction::Pseudonymize => write!(f, "pseudonymize"),
            RedactionAction::Remove => write!(f, "remove"),
            RedactionAction::Reveal => write!(f, "reveal"),
            RedactionAction::Custom(template) => write!(f, "custom:{template}"),
        }
    }
}

impl std::str::FromStr for RedactionAction {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "mask" => Ok(RedactionAction::Mask),
            "hash" => Ok(RedactionAction::Hash),
            "pseudonymize" => Ok(RedactionAction::Pseudonymize),
            "remove" => Ok(RedactionAction::Remove),
            "reveal" => Ok(RedactionAction::Reveal),
            other => match other.strip_prefix("custom:") {
                Some(template) => Ok(RedactionAction::Custom(template.to_owned())),
                None => Err(format!("unknown redaction action '{other}'")),
            },
        }
    }
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
/// EntitySource enum
pub enum EntitySource {
    /// Detected by regex patterns.
    Regex,
    /// Detected by ML model.
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

impl std::str::FromStr for EntitySource {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "regex" => Ok(EntitySource::Regex),
            "model" => Ok(EntitySource::Model),
            other => Err(format!("unknown entity source '{other}'")),
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
/// AuditEntry struct
pub struct AuditEntry {
    /// Unique identifier for the entry.
    pub id: String,
    /// ISO-8601 timestamp of creation.
    pub timestamp: String,
    /// PII category name.
    pub category: String,
    /// Redaction action applied.
    pub action: RedactionAction,
    /// blake3 digest of the original span.
    pub span_hash: String,
    /// Length of the original span in characters.
    pub span_length: u32,
    /// Detection confidence, if applicable.
    pub confidence: Option<f32>,
    /// Detector source.
    pub source: EntitySource,
    /// Pipeline version used.
    pub pipeline_version: String,
    /// Config hash the entry was minted under.
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
    /// The Tier 0 schema vertical active when this entity was detected, or `None` when
    /// no vertical was configured.
    ///
    /// The recorded value is `"<id>@<digest>"` — see
    /// [`VerticalConfig::provenance_id`](crate::pii::VerticalConfig::provenance_id) — not
    /// the bare vertical id, because an id alone is a false provenance claim: the same
    /// id with a different label set detects different things, and this field's job is
    /// to say what *was* detectable. Covered by [`compute_chain_hash`], so it cannot be
    /// rewritten after the fact without breaking verification. `None` hashes as the
    /// empty string — which is exactly what entries written before this field existed
    /// hashed as, so older chains still verify.
    #[serde(default)]
    pub vertical: Option<String>,
    /// The model that produced this entity, or `None` for regex-only entries or
    /// chains written before this field existed. Recorded as
    /// `"<model_id>@<digest8>"` — see `VerticalConfig::provenance_id` pattern —
    /// covered by [`compute_chain_hash`] so it cannot be rewritten without
    /// breaking verification. `None` hashes as no bytes, so older chains verify.
    #[serde(default)]
    pub model: Option<String>,
    /// blake3 over the previous chain hash and this entry's identifying fields.
    pub chain_hash: String,
}

/// Everything needed to mint an [`AuditEntry`] except its position in the chain.
#[derive(Debug, Clone)]
/// AuditEntryInput struct
pub struct AuditEntryInput {
    /// Unique identifier for the entry.
    pub id: String,
    /// PII category name.
    pub category: String,
    /// Redaction action applied.
    pub action: RedactionAction,
    /// blake3 digest of the original span.
    pub span_hash: String,
    /// Length of the original span.
    pub span_length: u32,
    /// Detection confidence.
    pub confidence: Option<f32>,
    /// Detector source.
    pub source: EntitySource,
    /// Pipeline version.
    pub pipeline_version: String,
    /// Config hash.
    pub config_hash: String,
    /// See [`AuditEntry::principal`].
    pub principal: Option<String>,
    /// See [`AuditEntry::vertical`].
    pub vertical: Option<String>,
    /// See [`AuditEntry::model`].
    pub model: Option<String>,
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
                vertical: input.vertical.as_deref(),
                model: input.model.as_deref(),
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
            vertical: input.vertical,
            model: input.model,
            chain_hash,
        }
    }

    /// The fields of this entry that [`compute_chain_hash`] covers.
    pub(crate) fn chain_hash_fields(&self) -> ChainHashFields<'_> {
        ChainHashFields {
            id: &self.id,
            category: &self.category,
            action: &self.action,
            span_hash: &self.span_hash,
            config_hash: &self.config_hash,
            principal: self.principal.as_deref(),
            vertical: self.vertical.as_deref(),
            model: self.model.as_deref(),
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
///
/// `pub(crate)`: grepping the whole workspace confirms nothing outside this crate
/// constructs this type today — the only caller is `AuditEntry::chain_hash_fields`
/// via `audit::chain`, both in this crate. There is no public constructor, so keeping
/// this `pub` would only ever be exercised through a struct literal — reintroducing
/// exactly the positional-argument-style transposition risk the named-field design
/// above exists to avoid. Narrowing visibility instead of adding a constructor keeps
/// that guarantee intact without giving external crates a way to build one at all.
#[derive(Debug, Clone, Copy)]
/// ChainHashFields struct
pub struct ChainHashFields<'a> {
    pub id: &'a str,
    pub category: &'a str,
    pub action: &'a RedactionAction,
    pub span_hash: &'a str,
    pub config_hash: &'a str,
    pub principal: Option<&'a str>,
    pub vertical: Option<&'a str>,
    pub model: Option<&'a str>,
}

/// Tag byte prepended to the vertical's length-prefixed bytes when present.
///
/// `0xff` can never occur in a valid UTF-8 `&str` (it is not a valid byte in any
/// position of a UTF-8 encoding), so using it as a presence tag — followed by an
/// explicit little-endian length prefix — makes the framing of the `vertical` field
/// unambiguous regardless of what the string itself contains. Without this, hashing
/// `principal` and `vertical` back-to-back with no delimiter meant
/// `principal="P1", vertical="V1"` and `principal="P1V", vertical="1"` hashed
/// identically, since blake3's streaming `.update()` is equivalent to hashing the
/// concatenation of everything fed to it.
const VERTICAL_PRESENT_TAG: u8 = 0xff;
const MODEL_PRESENT_TAG: u8 = 0xfe;

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
    // Same rule, same reason, one field later: absent vertical provenance hashes as no
    // bytes at all, so chains written before this field existed continue to verify
    // byte-for-byte. Appended *after* principal — never reorder these two hashes.
    //
    // Unlike `principal`, a present vertical is framed with a tag byte and an explicit
    // length prefix (see `VERTICAL_PRESENT_TAG`) so its boundary with the preceding
    // `principal` bytes — and its own content — can never be shifted undetected.
    if let Some(vertical) = fields.vertical {
        hasher.update(&[VERTICAL_PRESENT_TAG]);
        hasher.update(&(vertical.len() as u64).to_le_bytes());
        hasher.update(vertical.as_bytes());
    }
    if let Some(model) = fields.model {
        hasher.update(&[MODEL_PRESENT_TAG]);
        hasher.update(&(model.len() as u64).to_le_bytes());
        hasher.update(model.as_bytes());
    }
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
            vertical: None,
            model: None,
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
            vertical: None,
            model: None,
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

    /// Absent vertical provenance must hash as nothing at all, not as a sentinel
    /// string, so chains written before the field existed still verify.
    ///
    /// Unlike the principal field, this can no longer be demonstrated by comparing
    /// against `Some("")` — the tagged framing that fixes the boundary-ambiguity bug
    /// means a *present* empty vertical now hashes differently from an absent one (see
    /// `should_hash_a_present_empty_vertical_differently_from_an_absent_one`). So this
    /// test instead confirms the "no bytes at all" property directly, by replicating
    /// the hasher's byte sequence up to (and including) `principal` and checking that
    /// `compute_chain_hash` with `vertical: None` produces exactly that hash, with
    /// nothing appended for `vertical`.
    #[test]
    fn should_hash_an_absent_vertical_as_no_bytes() {
        let fields = ChainHashFields {
            id: "id-1",
            category: "Email",
            action: &RedactionAction::Mask,
            span_hash: "abc",
            config_hash: "cfg",
            principal: None,
            vertical: None,
            model: None,
        };

        let mut hasher = blake3::Hasher::new();
        hasher.update(b"prev");
        hasher.update(&0u64.to_le_bytes());
        hasher.update(fields.id.as_bytes());
        hasher.update(fields.category.as_bytes());
        let action_str = serde_json::to_string(fields.action).unwrap_or_default();
        hasher.update(action_str.as_bytes());
        hasher.update(fields.span_hash.as_bytes());
        hasher.update(fields.config_hash.as_bytes());
        hasher.update(fields.principal.unwrap_or("").as_bytes());
        // Deliberately nothing appended here for `vertical` — that is the property
        // under test.
        let expected_with_no_vertical_bytes = hasher.finalize().to_hex().to_string();

        assert_eq!(
            compute_chain_hash("prev", 0, fields),
            expected_with_no_vertical_bytes
        );
    }

    /// The literal captured in Task 0, *before* the `vertical` field existed, against
    /// the exact `input("id-1")` fixture defined above. If this test ever needs to
    /// change, the chain-hash byte layout changed in a way that breaks every audit
    /// chain written before this field existed — that is not a refactor, it is an
    /// incompatible format change and must be treated as one.
    #[test]
    fn should_verify_a_chain_written_before_the_vertical_field_existed() {
        let entry = AuditEntry::new(input("id-1"), "prev", 0);
        assert!(
            entry.vertical.is_none(),
            "the fixture must not set a vertical for this test to be meaningful"
        );
        assert_eq!(
            entry.chain_hash,
            "72eb1d2f14c701e0c58280b2d7fc5132fdc0564a3ed42e7b0c4b84cfdd5a3ee4"
        );
    }

    /// Since the tagged framing was introduced, `Some("")` is no longer
    /// indistinguishable from `None` — the tag and zero-length prefix bytes are still
    /// emitted for `Some("")`, while `None` emits nothing at all.
    #[test]
    fn should_hash_a_present_empty_vertical_differently_from_an_absent_one() {
        let fields = ChainHashFields {
            id: "id-1",
            category: "Email",
            action: &RedactionAction::Mask,
            span_hash: "abc",
            config_hash: "cfg",
            principal: None,
            vertical: None,
            model: None,
        };
        let empty_string_vertical = ChainHashFields {
            vertical: Some(""),
            ..fields
        };
        assert_ne!(
            compute_chain_hash("prev", 0, fields),
            compute_chain_hash("prev", 0, empty_string_vertical)
        );
    }

    /// Before the tagged framing was introduced, hashing `principal` and `vertical`
    /// back-to-back with no delimiter meant these two distinct `(principal, vertical)`
    /// pairs — whose naive concatenation is identical ("abc" either way) — hashed to
    /// the same chain hash. The length-prefixed framing must make them differ.
    #[test]
    fn should_not_collide_when_principal_and_vertical_bytes_shift_across_the_boundary() {
        let a = AuditEntry::new(
            AuditEntryInput {
                principal: Some("ab".into()),
                vertical: Some("c".into()),
                ..input("id-1")
            },
            "prev",
            0,
        );
        let b = AuditEntry::new(
            AuditEntryInput {
                principal: Some("a".into()),
                vertical: Some("bc".into()),
                ..input("id-1")
            },
            "prev",
            0,
        );
        assert_ne!(a.chain_hash, b.chain_hash);
    }

    #[test]
    fn should_change_the_chain_hash_when_the_vertical_changes() {
        let without_vertical = AuditEntry::new(input("id-1"), "prev", 0);
        let with_vertical = AuditEntry::new(
            AuditEntryInput {
                vertical: Some("finance@3f9a1c02".into()),
                ..input("id-1")
            },
            "prev",
            0,
        );
        assert_ne!(without_vertical.chain_hash, with_vertical.chain_hash);

        let with_different_vertical = AuditEntry::new(
            AuditEntryInput {
                vertical: Some("finance@aaaaaaaa".into()),
                ..input("id-1")
            },
            "prev",
            0,
        );
        assert_ne!(with_vertical.chain_hash, with_different_vertical.chain_hash);
    }

    #[test]
    fn should_round_trip_an_entry_with_a_vertical_through_json() {
        let entry = AuditEntry::new(
            AuditEntryInput {
                vertical: Some("finance@3f9a1c02".into()),
                ..input("id-1")
            },
            "prev",
            0,
        );
        let json = serde_json::to_string(&entry).unwrap();
        let round_tripped: AuditEntry = serde_json::from_str(&json).unwrap();
        assert_eq!(round_tripped.vertical, Some("finance@3f9a1c02".to_string()));
        assert_eq!(round_tripped.chain_hash, entry.chain_hash);
    }

    /// `#[serde(default)]` so an entry serialised before this field existed still
    /// deserialises, with `vertical: None`.
    #[test]
    fn should_deserialize_an_entry_with_no_vertical_key_at_all() {
        let json = serde_json::json!({
            "id": "id-1",
            "timestamp": "2026-01-01T00:00:00Z",
            "category": "Email",
            "action": "mask",
            "span_hash": "abc",
            "span_length": 10,
            "confidence": 1.0,
            "source": "regex",
            "pipeline_version": "1.0",
            "config_hash": "cfg",
            "chain_hash": "deadbeef",
        });
        let entry: AuditEntry = serde_json::from_value(json).unwrap();
        assert_eq!(entry.vertical, None);
        assert_eq!(entry.principal, None);
    }

    // The mode-to-action mapping moved to `RedactionEngine::audit_action` along with the
    // `From` impl it used to test; coverage lives in
    // `redaction::engine::tests::should_record_the_applied_action_for_every_mode`.

    #[test]
    fn should_change_the_chain_hash_when_the_model_changes() {
        let without_model = AuditEntry::new(input("id-1"), "prev", 0);
        let with_model = AuditEntry::new(
            AuditEntryInput {
                model: Some("fastino/gliner2-privacy-filter-PII-multi@a1b2c3d4".into()),
                ..input("id-1")
            },
            "prev",
            0,
        );
        assert_ne!(without_model.chain_hash, with_model.chain_hash);

        let with_different_model = AuditEntry::new(
            AuditEntryInput {
                model: Some("jamon8888/gliner2-guardrails-pii-f16@53c73fff".into()),
                ..input("id-1")
            },
            "prev",
            0,
        );
        assert_ne!(with_model.chain_hash, with_different_model.chain_hash);
    }

    #[test]
    fn should_round_trip_an_entry_with_a_model_through_json() {
        let entry = AuditEntry::new(
            AuditEntryInput {
                model: Some("fastino/gliner2-privacy-filter-PII-multi@a1b2c3d4".into()),
                ..input("id-1")
            },
            "prev",
            0,
        );
        let json = serde_json::to_string(&entry).unwrap();
        let round_tripped: AuditEntry = serde_json::from_str(&json).unwrap();
        assert_eq!(
            round_tripped.model,
            Some("fastino/gliner2-privacy-filter-PII-multi@a1b2c3d4".to_string())
        );
        assert_eq!(round_tripped.chain_hash, entry.chain_hash);
    }
}
