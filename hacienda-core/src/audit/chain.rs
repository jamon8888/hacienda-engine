//! Append-only, tamper-evident chain of [`AuditEntry`] records.

use crate::audit::entry::{compute_chain_hash, AuditEntry, AuditEntryInput};
use crate::audit::error::AuditError;

/// Chain hash that precedes the first entry.
pub const GENESIS_HASH: &str = "0000000000000000000000000000000000000000000000000000000000000000";

#[derive(Debug)]
/// Append-only, tamper-evident chain of audit entries.
pub struct AuditChain {
    entries: Vec<AuditEntry>,
    last_chain_hash: String,
    seq: u64,
    config_hash: String,
}

impl AuditChain {
    /// Create a new chain anchored to `config_hash`.
    pub fn new(config_hash: impl Into<String>) -> Self {
        Self {
            entries: Vec::new(),
            last_chain_hash: GENESIS_HASH.to_string(),
            seq: 0,
            config_hash: config_hash.into(),
        }
    }

    /// Mint an entry at the chain's current tip and append it.
    ///
    /// This is the entry point callers should use — it guarantees the sequence number
    /// and predecessor hash are consistent, which [`append`](Self::append) only verifies.
    /// `input.config_hash` is overwritten with the chain's own config hash.
    pub fn push(&mut self, mut input: AuditEntryInput) -> &AuditEntry {
        input.config_hash = self.config_hash.clone();
        let entry = AuditEntry::new(input, &self.last_chain_hash, self.seq);
        self.last_chain_hash = entry.chain_hash.clone();
        self.seq += 1;
        self.entries.push(entry);
        self.entries.last().expect("just pushed")
    }

    /// Append a pre-minted entry, rejecting it if it does not extend this chain.
    ///
    /// # Errors
    ///
    /// - [`AuditError::ConfigMismatch`] if the entry was minted under a different config.
    /// - [`AuditError::ChainIntegrity`] if the entry's chain hash does not follow the tip.
    pub fn append(&mut self, entry: AuditEntry) -> Result<(), AuditError> {
        if entry.config_hash != self.config_hash {
            return Err(AuditError::ConfigMismatch {
                expected: self.config_hash.clone(),
                actual: entry.config_hash,
            });
        }

        let expected_hash =
            compute_chain_hash(&self.last_chain_hash, self.seq, entry.chain_hash_fields());

        if entry.chain_hash != expected_hash {
            return Err(AuditError::ChainIntegrity {
                index: self.seq,
                expected: expected_hash,
                actual: entry.chain_hash,
            });
        }

        self.last_chain_hash = entry.chain_hash.clone();
        self.seq += 1;
        self.entries.push(entry);
        Ok(())
    }

    /// Recompute every chain hash from genesis and report the first mismatch.
    ///
    /// # Errors
    ///
    /// Returns [`AuditError::ChainIntegrity`] naming the index of the first entry
    /// whose recorded hash does not match its recomputed value.
    pub fn verify(&self) -> Result<(), AuditError> {
        verify_entries(&self.entries)
    }

    /// Return all entries in order.
    pub fn entries(&self) -> &[AuditEntry] {
        &self.entries
    }

    /// Number of entries in the chain.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the chain contains no entries.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Config hash the chain was created with.
    pub fn config_hash(&self) -> &str {
        &self.config_hash
    }

    /// Chain hash of the most recent entry, or [`GENESIS_HASH`] when empty.
    pub fn tip(&self) -> &str {
        &self.last_chain_hash
    }
}

/// Recompute every chain hash in `entries` from genesis and report the first mismatch.
///
/// The slice-only counterpart of [`AuditChain::verify`], for callers holding a
/// deserialized `Vec<AuditEntry>` (e.g. `hacienda-cli`'s `audit verify`, reading back a
/// flat JSON export) without also holding an `AuditChain`. Each entry's own `config_hash`
/// is already part of [`AuditEntry::chain_hash_fields`](crate::audit::entry::AuditEntry::chain_hash_fields),
/// so no chain-level config hash is needed here — this is exactly the loop
/// [`AuditChain::verify`] runs internally.
///
/// # Errors
///
/// Returns [`AuditError::ChainIntegrity`] naming the index of the first entry whose
/// recorded hash does not match its recomputed value.
pub fn verify_entries(entries: &[AuditEntry]) -> Result<(), AuditError> {
    let mut prev_hash = GENESIS_HASH.to_string();

    for (index, entry) in entries.iter().enumerate() {
        let expected_hash = compute_chain_hash(&prev_hash, index as u64, entry.chain_hash_fields());

        if entry.chain_hash != expected_hash {
            return Err(AuditError::ChainIntegrity {
                index: index as u64,
                expected: expected_hash,
                actual: entry.chain_hash.clone(),
            });
        }

        prev_hash = entry.chain_hash.clone();
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audit::entry::{EntitySource, RedactionAction};

    fn input(id: &str) -> AuditEntryInput {
        AuditEntryInput {
            id: id.into(),
            category: "Email".into(),
            action: RedactionAction::Mask,
            span_hash: blake3::hash(id.as_bytes()).to_hex().to_string(),
            span_length: 10,
            confidence: Some(1.0),
            source: EntitySource::Regex,
            pipeline_version: "1.0".into(),
            config_hash: String::new(),
            principal: None,
            vertical: None,
            model: None,
        }
    }

    #[test]
    fn should_start_empty_at_genesis() {
        let chain = AuditChain::new("cfg");
        assert!(chain.is_empty());
        assert_eq!(chain.tip(), GENESIS_HASH);
        assert!(chain.verify().is_ok());
    }

    #[test]
    fn should_verify_a_chain_built_with_push() {
        let mut chain = AuditChain::new("cfg");
        for i in 0..5 {
            chain.push(input(&format!("id-{i}")));
        }
        assert_eq!(chain.len(), 5);
        assert!(chain.verify().is_ok());
    }

    #[test]
    fn should_advance_the_tip_on_every_push() {
        let mut chain = AuditChain::new("cfg");
        chain.push(input("a"));
        let first_tip = chain.tip().to_string();
        chain.push(input("b"));
        assert_ne!(chain.tip(), first_tip);
    }

    #[test]
    fn should_detect_a_tampered_entry() {
        let mut chain = AuditChain::new("cfg");
        chain.push(input("a"));
        chain.push(input("b"));
        chain.entries[1].category = "CreditCard".into();

        match chain.verify() {
            Err(AuditError::ChainIntegrity { index, .. }) => assert_eq!(index, 1),
            other => panic!("expected a chain integrity error at index 1, got {other:?}"),
        }
    }

    /// Vertical provenance is inside the chain hash exactly like `principal` and
    /// `category` — rewriting it at rest must invalidate the entry, or the audit trail
    /// would answer "which vertical detected this" with a field an operator could edit
    /// after the fact.
    #[test]
    fn should_reject_a_tampered_vertical_id() {
        let mut chain = AuditChain::new("cfg");
        chain.push(AuditEntryInput {
            vertical: Some("finance@3f9a1c02".into()),
            ..input("a")
        });
        chain.entries[0].vertical = Some("finance@ffffffff".into());

        match chain.verify() {
            Err(AuditError::ChainIntegrity { index, .. }) => assert_eq!(index, 0),
            other => panic!("expected a chain integrity error at index 0, got {other:?}"),
        }
    }

    #[test]
    fn should_reject_an_entry_minted_under_a_different_config() {
        let mut chain = AuditChain::new("cfg-a");
        let foreign = AuditEntry::new(
            AuditEntryInput {
                config_hash: "cfg-b".into(),
                ..input("a")
            },
            GENESIS_HASH,
            0,
        );
        assert!(matches!(
            chain.append(foreign),
            Err(AuditError::ConfigMismatch { .. })
        ));
    }

    #[test]
    fn should_reject_an_entry_that_does_not_follow_the_tip() {
        let mut chain = AuditChain::new("cfg");
        chain.push(input("a"));
        // Minted against genesis, but the tip has already moved past it.
        let stale = AuditEntry::new(
            AuditEntryInput {
                config_hash: "cfg".into(),
                ..input("b")
            },
            GENESIS_HASH,
            0,
        );
        assert!(matches!(
            chain.append(stale),
            Err(AuditError::ChainIntegrity { .. })
        ));
    }

    #[test]
    fn should_accept_an_entry_minted_against_the_current_tip() {
        let mut chain = AuditChain::new("cfg");
        chain.push(input("a"));
        let next = AuditEntry::new(
            AuditEntryInput {
                config_hash: "cfg".into(),
                ..input("b")
            },
            chain.tip(),
            chain.len() as u64,
        );
        chain.append(next).expect("entry extends the chain");
        assert_eq!(chain.len(), 2);
        assert!(chain.verify().is_ok());
    }
}
