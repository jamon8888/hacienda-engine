//! Segment model for the audit chain — one writer's contiguous run of entries.
//!
//! A [`Segment`] wraps an [`AuditChain`] with a writer identity ([`NodeId`]) so that two
//! writers — say a CLI invocation and a server replica — can each own an independent chain
//! and still be verified together. A hash chain is inherently sequential: two appenders
//! need the same predecessor hash, so they cannot share one chain without serialising
//! through a single writer.
//!
//! Each segment's chain starts fresh at `GENESIS_HASH`. That is not a stylistic choice —
//! [`AuditChain::verify`] recomputes every entry hash using the entry's *array index* as
//! the sequence number, so a chain that began part-way through a global sequence would
//! fail its own verification. The link *between* segments therefore cannot live inside the
//! entry hashes; it lives in the [`SegmentSeal`] hash chain, one level up.

use chrono::Utc;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::audit::chain::AuditChain;
use crate::audit::entry::{AuditEntry, AuditEntryInput};
use crate::audit::error::AuditError;
use crate::tenancy::TenantId;

// ── NodeId ────────────────────────────────────────────────────────────────────

/// Identifies the writer that owns a segment.
///
/// Two concurrent writers (a CLI run and a server replica, or two server replicas) must
/// own separate segments so their append-only chains never interleave. `NodeId` is the
/// discriminator that makes their segments distinguishable in a shared store. It is
/// stored verbatim in every [`SegmentSeal`] so that the provenance of each segment is
/// self-describing after the writer exits.
///
/// `NodeId::default()` is `hostname-pid`. If `/etc/hostname` is unreadable and the
/// `HOSTNAME` environment variable is absent, the host component falls back to the
/// literal string `"unknown-host"` — stable within a run but not unique across machines.
/// Callers that need strong uniqueness should supply an explicit identifier via
/// [`NodeId::new`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NodeId(String);

impl NodeId {
    /// Create a `NodeId` from an explicit string — recommended when the caller controls
    /// the identity (tests, server configurations, multi-tenant deployments).
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    /// The underlying string identifier.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Default for NodeId {
    /// Hostname-pid. If the hostname cannot be determined, falls back to
    /// `"unknown-host-<pid>"` — stable within a process but not globally unique.
    fn default() -> Self {
        let host = resolve_hostname();
        let pid = process_id();
        Self(format!("{host}-{pid}"))
    }
}

/// The current process id, or a fixed placeholder on wasm32-unknown-unknown.
///
/// `std::process::id()` *compiles* on wasm32-unknown-unknown but panics at runtime
/// ("no pids on this platform") — one more of the wasm32 silent-failure surfaces Track
/// L3 exists to close, alongside the clock and `uuid` randomness ones. A browser tab has
/// no OS process to report, and — unlike the clock or randomness — there is no
/// JS-hosted equivalent to bind to; `0` is honest about that rather than fabricating a
/// number that looks like a real pid.
#[cfg(not(target_arch = "wasm32"))]
fn process_id() -> u32 {
    std::process::id()
}

#[cfg(target_arch = "wasm32")]
fn process_id() -> u32 {
    0
}

impl std::fmt::Display for NodeId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// Read the hostname without adding a dependency.
///
/// Tries `/etc/hostname` first (Linux), then `HOSTNAME` env-var (set by most shells),
/// then falls back to `"unknown-host"`. The fallback is documented in [`NodeId`]'s
/// `Default` impl comment.
fn resolve_hostname() -> String {
    if let Ok(contents) = std::fs::read_to_string("/etc/hostname") {
        let trimmed = contents.trim();
        if !trimmed.is_empty() {
            return trimmed.to_owned();
        }
    }
    if let Ok(val) = std::env::var("HOSTNAME") {
        if !val.is_empty() {
            return val;
        }
    }
    "unknown-host".to_owned()
}

// ── Segment ───────────────────────────────────────────────────────────────────

/// One writer's contiguous run of audit entries, sealed when the writer stops.
///
/// A `Segment` owns an [`AuditChain`] that starts at `GENESIS_HASH` with `seq = 0`.
/// When writing is done, call [`seal`](Self::seal) to produce a [`SegmentSeal`] whose
/// `seal_hash` commits to all segment metadata including `prev_seal_hash` from the
/// predecessor, making any later deletion of the predecessor detectable.
#[derive(Debug)]
pub struct Segment {
    /// uuid v4 string — unique per segment, stored in the seal for reference.
    segment_id: String,
    /// The tenant this segment belongs to (S1 spec §5: segments are named `(tenant,
    /// node)`, not just `node`). Defaults to the pre-S1 `default` tenant via
    /// [`open`](Self::open) — real per-tenant construction goes through
    /// [`open_for_tenant`](Self::open_for_tenant), unused until the stores that build
    /// `Segment`s are themselves threaded with a real `TenantCtx` (tracked separately;
    /// see the Vague 2 plan's S1 task notes).
    tenant_id: TenantId,
    node_id: NodeId,
    config_hash: String,
    /// Seal hash of the immediately preceding segment on this node, or `None` for the
    /// first segment. It is committed into `seal_hash` at seal time so that dropping
    /// the predecessor and blanking this field is detectable.
    prev_seal_hash: Option<String>,
    /// Entry chain for this segment — always starts at genesis.
    chain: AuditChain,
    /// RFC 3339 timestamp when this segment was opened.
    opened_at: String,
}

impl Segment {
    /// Open a new segment, inheriting the previous node's seal hash for linking.
    ///
    /// `prev_seal_hash` must be `None` only for the first segment this node ever opens.
    /// For all subsequent segments, pass the `seal_hash` from the previous
    /// [`SegmentSeal`] — omitting it leaves the chain truncatable without detection.
    pub fn open(
        node_id: NodeId,
        config_hash: impl Into<String>,
        prev_seal_hash: Option<String>,
    ) -> Self {
        Self::open_for_tenant(
            TenantId::default_tenant(),
            node_id,
            config_hash,
            prev_seal_hash,
        )
    }

    /// As [`open`](Self::open), scoped to an explicit tenant (S1 spec §5).
    pub fn open_for_tenant(
        tenant_id: TenantId,
        node_id: NodeId,
        config_hash: impl Into<String>,
        prev_seal_hash: Option<String>,
    ) -> Self {
        Self::open_with_id(
            Uuid::new_v4().to_string(),
            tenant_id,
            node_id,
            config_hash,
            prev_seal_hash,
        )
    }

    /// Open a segment with an explicit `segment_id`.
    ///
    /// This is the recovery counterpart to [`open`](Self::open): during recovery a
    /// [`FileAuditStore`](crate::audit::FileAuditStore) replays an existing `.jsonl` file
    /// and must produce a seal whose `segment_id` matches the filename. Using `open` would
    /// mint a new uuid v4, which would not match the file the entries came from.
    pub(crate) fn open_with_id(
        segment_id: impl Into<String>,
        tenant_id: TenantId,
        node_id: NodeId,
        config_hash: impl Into<String>,
        prev_seal_hash: Option<String>,
    ) -> Self {
        let config_hash = config_hash.into();
        Self {
            segment_id: segment_id.into(),
            tenant_id,
            node_id,
            chain: AuditChain::new(config_hash.clone()),
            config_hash,
            prev_seal_hash,
            opened_at: Utc::now().to_rfc3339(),
        }
    }

    /// Mint an entry at the current tip and append it.
    ///
    /// Delegates to [`AuditChain::push`], which stamps the chain's own config hash onto
    /// the input. Segment and chain are given the same config hash at
    /// [`open`](Self::open), so a caller cannot put the two out of step.
    pub fn push(&mut self, input: AuditEntryInput) -> &AuditEntry {
        self.chain.push(input)
    }

    /// Seal this segment, producing an immutable [`SegmentSeal`] that commits to all
    /// segment metadata. The `Segment` is consumed — no further entries can be appended.
    pub fn seal(self) -> SegmentSeal {
        let sealed_at = Utc::now().to_rfc3339();
        let sealed_tip = self.chain.tip().to_owned();
        let entry_count = self.chain.len() as u64;
        let tenant_id_str = self.tenant_id.as_str().to_owned();
        let node_id_str = self.node_id.as_str().to_owned();

        let seal_hash = compute_seal_hash(
            self.prev_seal_hash.as_deref(),
            &self.segment_id,
            &tenant_id_str,
            &node_id_str,
            &self.config_hash,
            &sealed_tip,
            entry_count,
            &self.opened_at,
            &sealed_at,
        );

        SegmentSeal {
            segment_id: self.segment_id,
            tenant_id: tenant_id_str,
            node_id: node_id_str,
            config_hash: self.config_hash,
            prev_seal_hash: self.prev_seal_hash,
            sealed_tip,
            entry_count,
            opened_at: self.opened_at,
            sealed_at,
            seal_hash,
        }
    }

    /// Unique identifier for this segment.
    pub fn id(&self) -> &str {
        &self.segment_id
    }

    /// The node that owns this segment.
    pub fn node_id(&self) -> &NodeId {
        &self.node_id
    }

    /// The tenant this segment belongs to.
    pub fn tenant_id(&self) -> &TenantId {
        &self.tenant_id
    }

    /// Chain hash of the most recent entry, or `GENESIS_HASH` when empty.
    pub fn tip(&self) -> &str {
        self.chain.tip()
    }

    /// Number of entries pushed so far.
    pub fn len(&self) -> usize {
        self.chain.len()
    }

    /// `true` when no entries have been pushed.
    pub fn is_empty(&self) -> bool {
        self.chain.is_empty()
    }

    /// All entries in this segment, in push order.
    pub fn entries(&self) -> &[AuditEntry] {
        self.chain.entries()
    }

    /// Append a pre-minted entry read back from storage, rejecting it if it does not extend
    /// this segment's chain.
    ///
    /// This is the recovery counterpart to [`push`](Self::push): `push` mints a new entry,
    /// this validates one that already exists on disk. Delegating to
    /// [`AuditChain::append`] means a replayed file that was edited on disk fails at load
    /// time rather than being silently adopted into a live chain — which is the property
    /// that makes `FileAuditStore::open` fail on a tampered `.jsonl`.
    ///
    /// # Errors
    ///
    /// - [`AuditError::ConfigMismatch`] — the entry was minted under a different config.
    /// - [`AuditError::ChainIntegrity`] — the entry's chain hash does not follow the tip.
    pub fn append(&mut self, entry: AuditEntry) -> Result<(), AuditError> {
        self.chain.append(entry)
    }

    /// Verify the internal entry chain — delegates to [`AuditChain::verify`], unchanged.
    ///
    /// This confirms that no entry within this segment was tampered with. It says nothing
    /// about the link to adjacent segments; use [`verify_seal_chain`] for that.
    pub fn verify(&self) -> Result<(), AuditError> {
        self.chain.verify()
    }
}

// ── SegmentSeal ───────────────────────────────────────────────────────────────

/// The immutable record left behind when a segment is closed.
///
/// `seal_hash` commits to every other field, including `prev_seal_hash`, so that a node's
/// seals form a hash chain of segments — the same construction as the entry chain, one
/// level up.
///
/// That back-pointer is what makes truncation detectable, and truncation is the attack the
/// segment layer would otherwise be wide open to. Without it, an attacker could delete a
/// node's oldest segment, blank the successor's `prev_seal_hash`, and present the remainder
/// as a complete history: every surviving segment verifies internally, and every link
/// between survivors holds. Suppressing the earliest records is precisely what a
/// tamper-evident log exists to prevent, so the cost — one hash input — is not worth
/// arguing about.
///
/// `entry_count` is in the preimage as a cheaper, more legible integrity signal rather than
/// as a distinct security property. `sealed_tip` already changes when entries are removed,
/// since the tip *is* the last entry's chain hash. What `entry_count` adds is an invariant a
/// store can assert at recovery — replayed entries must number exactly this many — which
/// fails with "expected 40 entries, found 37" instead of an opaque hash mismatch.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SegmentSeal {
    pub segment_id: String,
    /// The tenant this segment belongs to (S1 spec §5). Always `"default"` until the
    /// store that produced this seal is itself threaded with a real tenant context —
    /// see [`Segment::tenant_id`]'s doc comment.
    ///
    /// Defaulted on deserialize (mirroring `review::types::ReviewQueueItem::tenant_id`):
    /// a `*.seal.json` file written by any pre-S1 build has no `tenant_id` key at all, so
    /// without this, loading it back would fail outright with a missing-field error
    /// instead of defaulting to `"default"`.
    #[serde(default = "default_tenant_id_string")]
    pub tenant_id: String,
    pub node_id: String,
    pub config_hash: String,
    /// Seal hash of the immediately preceding segment on this node, or `None` for the
    /// first segment. Committed into `seal_hash` so deletion of the predecessor is
    /// detectable.
    pub prev_seal_hash: Option<String>,
    /// Entry chain tip at seal time.
    pub sealed_tip: String,
    /// Number of entries in the segment at seal time.
    pub entry_count: u64,
    /// RFC 3339 timestamp when the segment was opened.
    pub opened_at: String,
    /// RFC 3339 timestamp when the segment was sealed.
    pub sealed_at: String,
    /// blake3 over all other fields. Recompute with [`compute_seal_hash`] and compare to
    /// detect any post-seal modification.
    pub seal_hash: String,
}

fn default_tenant_id_string() -> String {
    crate::tenancy::DEFAULT_TENANT.to_owned()
}

// ── compute_seal_hash ─────────────────────────────────────────────────────────

/// Compute the blake3 hash that seals a segment's metadata.
///
/// Every field is length-prefixed before being fed to the hasher. This prevents
/// ambiguous splits: without length prefixes, `("ab", "c")` and `("a", "bc")` would
/// produce the same byte sequence. `compute_chain_hash` in `entry.rs` uses fixed-width
/// integers (which are self-delimiting) alongside string bytes and does not need this
/// guard, but the seal fields are all variable-length strings, so it is required here.
///
/// The field order and encoding here is the canonical definition of the seal hash for
/// this crate. Changing either breaks all stored seals.
///
/// The nine arguments map one-to-one to the nine `SegmentSeal` fields that the hash
/// covers. Wrapping them in a struct would force callers to construct one just to call this
/// function; keeping them flat is both more legible and consistent with `compute_chain_hash`.
#[allow(clippy::too_many_arguments)]
pub fn compute_seal_hash(
    prev_seal_hash: Option<&str>,
    segment_id: &str,
    tenant_id: &str,
    node_id: &str,
    config_hash: &str,
    sealed_tip: &str,
    entry_count: u64,
    opened_at: &str,
    sealed_at: &str,
) -> String {
    let mut hasher = blake3::Hasher::new();

    // Encode Option<&str> as a 1-byte presence flag followed by the length-prefixed
    // content. Using 0x00 / 0x01 as the flag is unambiguous because the content bytes
    // follow in a fixed pattern.
    match prev_seal_hash {
        None => {
            hasher.update(&[0u8]);
        }
        Some(h) => {
            hasher.update(&[1u8]);
            hash_str(&mut hasher, h);
        }
    }

    hash_str(&mut hasher, segment_id);
    hash_str(&mut hasher, tenant_id);
    hash_str(&mut hasher, node_id);
    hash_str(&mut hasher, config_hash);
    hash_str(&mut hasher, sealed_tip);
    hasher.update(&entry_count.to_le_bytes());
    hash_str(&mut hasher, opened_at);
    hash_str(&mut hasher, sealed_at);

    hasher.finalize().to_hex().to_string()
}

/// Feed a length-prefixed string into the hasher to prevent ambiguous splits.
fn hash_str(hasher: &mut blake3::Hasher, s: &str) {
    let bytes = s.as_bytes();
    hasher.update(&(bytes.len() as u64).to_le_bytes());
    hasher.update(bytes);
}

/// The pre-S1 seal hash: [`compute_seal_hash`] without `tenant_id` in the preimage.
///
/// S1 added `tenant_id` to `compute_seal_hash`'s preimage (S1 spec §5), but every seal
/// sealed before that migration has a `seal_hash` on disk that was computed without it —
/// migration 0005 backfills `tenant_id = "default"` onto those rows' *column*, but cannot
/// retroactively change the *hash* already committed to disk/database. Without this
/// fallback, `check_seal_integrity` would recompute a different hash for every
/// pre-existing seal and report `AuditError::SegmentIntegrity` — a false "tampered" on an
/// upgrading deployment's entire historical audit trail, not an actual tamper.
///
/// Only ever tried for a `"default"`-tenant seal (see [`check_seal_integrity`]) — S1
/// shipped with every existing deployment on the default tenant by construction (spec
/// §8), so a non-default tenant id can only belong to a segment sealed after this
/// migration, which must satisfy the current, tenant-aware hash.
#[allow(clippy::too_many_arguments)]
fn compute_legacy_seal_hash(
    prev_seal_hash: Option<&str>,
    segment_id: &str,
    node_id: &str,
    config_hash: &str,
    sealed_tip: &str,
    entry_count: u64,
    opened_at: &str,
    sealed_at: &str,
) -> String {
    let mut hasher = blake3::Hasher::new();

    match prev_seal_hash {
        None => {
            hasher.update(&[0u8]);
        }
        Some(h) => {
            hasher.update(&[1u8]);
            hash_str(&mut hasher, h);
        }
    }

    hash_str(&mut hasher, segment_id);
    hash_str(&mut hasher, node_id);
    hash_str(&mut hasher, config_hash);
    hash_str(&mut hasher, sealed_tip);
    hasher.update(&entry_count.to_le_bytes());
    hash_str(&mut hasher, opened_at);
    hash_str(&mut hasher, sealed_at);

    hasher.finalize().to_hex().to_string()
}

// ── verify_seal_chain ─────────────────────────────────────────────────────────

/// Verify a slice of [`SegmentSeal`]s from a single node, oldest first.
///
/// Two checks are applied to every seal:
///
/// 1. **Self-integrity** — recompute `seal_hash` from the seal's own fields and compare.
///    Any field mutation (including blanking `prev_seal_hash`) is detected here.
/// 2. **Link check** — confirm that `prev_seal_hash` matches the `seal_hash` of the
///    immediately preceding entry in the slice. An attacker who recomputes seal hashes
///    after splicing seals together would still break this check.
///
/// The two together are what defeat truncation. Dropping the oldest segment and blanking
/// the new first seal's `prev_seal_hash` alters a field the hash covers, so check 1 fires.
/// Recomputing the hash to match the blanked field satisfies check 1, but that changes the
/// seal's own hash, so check 2 fires on its successor. An attacker who repairs both has had
/// to re-mint every seal in the run — which is exactly the work a hash chain is meant to
/// force, and what an anchor over these seal hashes will eventually make impossible.
///
/// # Errors
///
/// - [`AuditError::SegmentIntegrity`] — a seal's `seal_hash` does not match its fields.
/// - [`AuditError::SegmentLink`] — a seal's `prev_seal_hash` does not match its
///   predecessor's `seal_hash`.
pub fn verify_seal_chain(seals: &[SegmentSeal]) -> Result<(), AuditError> {
    let mut predecessor: Option<&SegmentSeal> = None;

    for seal in seals {
        check_seal_integrity(seal)?;
        check_seal_link(seal, predecessor)?;
        predecessor = Some(seal);
    }

    Ok(())
}

/// Check that a seal's recorded hash matches what `compute_seal_hash` produces.
///
/// Falls back to [`compute_legacy_seal_hash`] for a `"default"`-tenant seal that doesn't
/// match the current (tenant-aware) formula — see that function's doc comment for why.
fn check_seal_integrity(seal: &SegmentSeal) -> Result<(), AuditError> {
    let expected = compute_seal_hash(
        seal.prev_seal_hash.as_deref(),
        &seal.segment_id,
        &seal.tenant_id,
        &seal.node_id,
        &seal.config_hash,
        &seal.sealed_tip,
        seal.entry_count,
        &seal.opened_at,
        &seal.sealed_at,
    );

    if seal.seal_hash == expected {
        return Ok(());
    }

    if seal.tenant_id == crate::tenancy::DEFAULT_TENANT {
        let legacy_expected = compute_legacy_seal_hash(
            seal.prev_seal_hash.as_deref(),
            &seal.segment_id,
            &seal.node_id,
            &seal.config_hash,
            &seal.sealed_tip,
            seal.entry_count,
            &seal.opened_at,
            &seal.sealed_at,
        );
        if seal.seal_hash == legacy_expected {
            return Ok(());
        }
    }

    Err(AuditError::SegmentIntegrity {
        segment_id: seal.segment_id.clone(),
        expected,
        actual: seal.seal_hash.clone(),
    })
}

/// Check that a seal's `prev_seal_hash` points to its predecessor's `seal_hash`.
///
/// `predecessor` is `None` only for the first seal in the run. Threading it through as a
/// value rather than indexing backwards keeps the "no predecessor" case a real state
/// rather than an out-of-bounds read that happens to return `None`.
fn check_seal_link(
    seal: &SegmentSeal,
    predecessor: Option<&SegmentSeal>,
) -> Result<(), AuditError> {
    // Whatever the predecessor's hash is, the seal must point at it. A missing predecessor
    // and a missing back-pointer are the same condition seen from two sides, so comparing
    // the two Options directly covers every combination without a catch-all arm.
    let expected = predecessor.map(|prev| prev.seal_hash.as_str());
    let actual = seal.prev_seal_hash.as_deref();

    if expected == actual {
        return Ok(());
    }

    Err(AuditError::SegmentLink {
        segment_id: seal.segment_id.clone(),
        expected: expected.unwrap_or("none (first segment)").to_owned(),
        actual: actual.unwrap_or("none").to_owned(),
    })
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audit::entry::{AuditEntryInput, EntitySource, RedactionAction};

    // ── helpers ──────────────────────────────────────────────────────────────

    fn node() -> NodeId {
        NodeId::new("test-node-0")
    }

    fn config() -> &'static str {
        "config-hash-abc"
    }

    fn make_input(id: &str) -> AuditEntryInput {
        AuditEntryInput {
            id: id.into(),
            category: "Email".into(),
            action: RedactionAction::Mask,
            span_hash: blake3::hash(id.as_bytes()).to_hex().to_string(),
            span_length: id.len() as u32,
            confidence: Some(0.99),
            source: EntitySource::Regex,
            pipeline_version: "1.0".into(),
            config_hash: config().into(),
            principal: None,
            vertical: None,
            model: None,
        }
    }

    // ── Step 6 tests ─────────────────────────────────────────────────────────

    #[test]
    fn should_open_a_segment_at_genesis_with_no_entries() {
        let seg = Segment::open(node(), config(), None);
        assert!(seg.is_empty());
        assert_eq!(seg.len(), 0);
        assert_eq!(seg.tip(), crate::audit::GENESIS_HASH);
    }

    #[test]
    fn should_carry_the_previous_seal_hash_into_the_new_segment() {
        let seg1 = Segment::open(node(), config(), None);
        let seal1 = seg1.seal();

        let seg2 = Segment::open(node(), config(), Some(seal1.seal_hash.clone()));
        let seal2 = seg2.seal();

        assert_eq!(
            seal2.prev_seal_hash.as_deref(),
            Some(seal1.seal_hash.as_str())
        );
    }

    #[test]
    fn should_verify_a_sealed_segment_internally() {
        let mut seg = Segment::open(node(), config(), None);
        seg.push(make_input("e1"));
        seg.push(make_input("e2"));
        seg.verify()
            .expect("two-entry segment must verify internally");
    }

    #[test]
    fn should_record_the_entry_count_in_the_seal() {
        let mut seg = Segment::open(node(), config(), None);
        for i in 0..3u32 {
            seg.push(make_input(&format!("e{i}")));
        }
        let seal = seg.seal();
        assert_eq!(seal.entry_count, 3);
    }

    #[test]
    fn should_default_new_segments_to_the_default_tenant() {
        let seg = Segment::open(node(), config(), None);
        assert_eq!(seg.tenant_id(), &crate::tenancy::TenantId::default_tenant());
        assert_eq!(seg.seal().tenant_id, "default");
    }

    #[test]
    fn should_scope_a_segment_to_an_explicit_tenant() {
        let seg = Segment::open_for_tenant(
            crate::tenancy::TenantId::new("acme"),
            node(),
            config(),
            None,
        );
        assert_eq!(seg.seal().tenant_id, "acme");
    }

    /// S1 spec §5: segments are named `(tenant, node)`, not just `node` — the seal
    /// hash must commit to `tenant_id` so that relabelling a segment onto a different
    /// tenant after the fact is detected exactly like any other seal tamper.
    #[test]
    fn should_detect_a_seal_relabelled_onto_a_different_tenant() {
        let (mut seals, _) = build_seal_chain(1);
        seals[0].tenant_id = "attacker-tenant".to_owned();
        let err = verify_seal_chain(&seals).expect_err("relabelled tenant_id must be rejected");
        assert!(
            matches!(err, AuditError::SegmentIntegrity { .. }),
            "expected SegmentIntegrity, got {err:?}"
        );
    }

    #[test]
    fn should_accept_a_correctly_linked_run_of_seals() {
        let (seals, _) = build_seal_chain(3);
        verify_seal_chain(&seals).expect("well-formed chain must verify");
    }

    #[test]
    fn should_reject_a_seal_whose_hash_does_not_cover_its_fields() {
        let (mut seals, _) = build_seal_chain(2);
        // Tamper with a field after the seal is created — the recorded seal_hash
        // now does not match what compute_seal_hash would produce.
        seals[0].entry_count += 1;
        let err = verify_seal_chain(&seals).expect_err("tampered seal must be rejected");
        assert!(
            matches!(err, AuditError::SegmentIntegrity { .. }),
            "expected SegmentIntegrity, got {err:?}"
        );
    }

    /// D2 truncation attack: build three seals, drop the oldest, set the new
    /// first segment's `prev_seal_hash` to `None`, and assert that verification
    /// still detects the deletion.
    ///
    /// If `prev_seal_hash` were absent from the `compute_seal_hash` preimage,
    /// the stored `seal_hash` would match the tampered fields (because the field
    /// that changed is not covered), and this test would pass against a broken
    /// implementation — which is why it exists.
    #[test]
    fn should_detect_a_deleted_leading_segment() {
        let (mut seals, _) = build_seal_chain(3);
        // Drop the first seal — the attacker pretends this node only has two segments.
        seals.remove(0);
        // Blank the new first seal's back-pointer to make it look like a genesis.
        seals[0].prev_seal_hash = None;
        // The recorded seal_hash on seals[0] was computed with a non-None prev_seal_hash,
        // so it no longer matches what compute_seal_hash produces from the tampered fields.
        let err = verify_seal_chain(&seals).expect_err("deleted leading segment must be detected");
        assert!(
            matches!(err, AuditError::SegmentIntegrity { .. }),
            "expected SegmentIntegrity, got {err:?}"
        );
    }

    #[test]
    fn should_detect_a_reordered_pair_of_seals() {
        let (mut seals, _) = build_seal_chain(3);
        seals.swap(1, 2);
        let err = verify_seal_chain(&seals).expect_err("reordered seals must be detected");
        // Could be SegmentIntegrity (bad hash) or SegmentLink (wrong predecessor).
        assert!(
            matches!(
                err,
                AuditError::SegmentIntegrity { .. } | AuditError::SegmentLink { .. }
            ),
            "expected SegmentIntegrity or SegmentLink, got {err:?}"
        );
    }

    /// An attacker who shortens a segment's entry run *and* re-seals it to match still
    /// loses, because re-sealing changes the hash the successor points at.
    ///
    /// This is the case that the deletion test above cannot reach: there, the tampered
    /// seal's own hash was left stale. Here it is repaired, so `SegmentIntegrity` passes
    /// and only the link check stands between the attacker and a clean verification.
    #[test]
    fn should_detect_entries_truncated_from_a_segment_tail() {
        let (mut seals, node_id) = build_seal_chain(2);

        // Re-mint the first segment with one fewer entry and seal it honestly, so its
        // own hash is internally consistent with the shortened run.
        let mut shortened = Segment::open(node_id, config(), None);
        shortened.push(make_input("chain-entry-0"));
        seals[0] = shortened.seal();

        check_seal_integrity(&seals[0]).expect("the re-minted seal is self-consistent");

        let err = verify_seal_chain(&seals).expect_err("a re-sealed segment breaks the link");
        assert!(
            matches!(err, AuditError::SegmentLink { .. }),
            "expected SegmentLink, got {err:?}"
        );
    }

    /// S1 added `tenant_id` to the seal hash preimage. A seal sealed before that
    /// migration has a `seal_hash` on disk computed *without* `tenant_id` — migration
    /// 0005 backfills the `tenant_id` *column* to `"default"`, but cannot rewrite the
    /// hash already committed. Such a seal must still verify (via
    /// `compute_legacy_seal_hash`'s fallback), or every pre-existing deployment's audit
    /// trail would report as tampered on the first `verify` after upgrading.
    #[test]
    fn should_accept_a_pre_s1_seal_hashed_without_tenant_id() {
        let legacy_hash = compute_legacy_seal_hash(
            None,
            "legacy-segment",
            node().as_str(),
            config(),
            "legacy-tip",
            2,
            "2026-01-01T00:00:00Z",
            "2026-01-01T00:01:00Z",
        );
        let legacy_seal = SegmentSeal {
            segment_id: "legacy-segment".to_owned(),
            tenant_id: crate::tenancy::DEFAULT_TENANT.to_owned(),
            node_id: node().as_str().to_owned(),
            config_hash: config().to_owned(),
            prev_seal_hash: None,
            sealed_tip: "legacy-tip".to_owned(),
            entry_count: 2,
            opened_at: "2026-01-01T00:00:00Z".to_owned(),
            sealed_at: "2026-01-01T00:01:00Z".to_owned(),
            seal_hash: legacy_hash,
        };

        check_seal_integrity(&legacy_seal).expect("a pre-S1 seal must still verify");
    }

    /// The legacy fallback must not become a second way to forge a seal: a genuinely
    /// tampered `"default"`-tenant seal has to fail both the current and the legacy
    /// formula, since tampering the same field breaks both.
    #[test]
    fn legacy_fallback_does_not_accept_a_genuinely_tampered_default_tenant_seal() {
        let legacy_hash = compute_legacy_seal_hash(
            None,
            "legacy-segment",
            node().as_str(),
            config(),
            "legacy-tip",
            2,
            "2026-01-01T00:00:00Z",
            "2026-01-01T00:01:00Z",
        );
        let mut tampered = SegmentSeal {
            segment_id: "legacy-segment".to_owned(),
            tenant_id: crate::tenancy::DEFAULT_TENANT.to_owned(),
            node_id: node().as_str().to_owned(),
            config_hash: config().to_owned(),
            prev_seal_hash: None,
            sealed_tip: "legacy-tip".to_owned(),
            entry_count: 2,
            opened_at: "2026-01-01T00:00:00Z".to_owned(),
            sealed_at: "2026-01-01T00:01:00Z".to_owned(),
            seal_hash: legacy_hash,
        };
        tampered.entry_count = 99;

        let err =
            check_seal_integrity(&tampered).expect_err("tampered legacy seal must be rejected");
        assert!(matches!(err, AuditError::SegmentIntegrity { .. }));
    }

    /// A `*.seal.json` written by a pre-S1 build has no `"tenant_id"` key at all —
    /// deserializing it must default to `"default"`, not fail with a missing-field
    /// error.
    #[test]
    fn should_default_tenant_id_when_deserializing_a_pre_s1_seal_file() {
        let json = r#"{
            "segment_id": "legacy-segment",
            "node_id": "legacy-node",
            "config_hash": "config-hash-abc",
            "prev_seal_hash": null,
            "sealed_tip": "legacy-tip",
            "entry_count": 2,
            "opened_at": "2026-01-01T00:00:00Z",
            "sealed_at": "2026-01-01T00:01:00Z",
            "seal_hash": "deadbeef"
        }"#;
        let seal: SegmentSeal = serde_json::from_str(json).expect("pre-S1 seal must still parse");
        assert_eq!(seal.tenant_id, crate::tenancy::DEFAULT_TENANT);
    }

    // ── helpers ──────────────────────────────────────────────────────────────

    /// Build `count` chained seals for a single node, returning the seals and the
    /// `NodeId` they were written under so a test can mint further segments on it.
    ///
    /// Each segment gets two entries so that a test can shorten one without emptying it.
    fn build_seal_chain(count: usize) -> (Vec<SegmentSeal>, NodeId) {
        let node_id = NodeId::new("chain-test-node");
        let mut prev_seal_hash: Option<String> = None;
        let mut seals = Vec::with_capacity(count);

        for i in 0..count {
            let mut seg = Segment::open(node_id.clone(), config(), prev_seal_hash.clone());
            seg.push(make_input(&format!("chain-entry-{i}")));
            seg.push(make_input(&format!("chain-entry-{i}-b")));
            let seal = seg.seal();
            prev_seal_hash = Some(seal.seal_hash.clone());
            seals.push(seal);
        }

        (seals, node_id)
    }
}
