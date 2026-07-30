//! The [`AuditStore`] trait and the [`InMemoryAuditStore`] backend.
//!
//! The trait is the persistence seam that lets the facade swap between an in-memory store
//! (default, same behaviour as today), a file backend (Task 3), and eventually a Postgres
//! backend (Phase 6) without any API change above it. [`async_trait`] keeps the trait
//! object-safe by boxing the returned futures, which is why [`Arc<dyn AuditStore>`] works
//! without native async-in-trait support.
//!
//! The audit store is deliberately batch-oriented: every call to [`AuditStore::append`]
//! commits a whole document's worth of entries at once. That is the right transaction
//! boundary for a file or database backend (one fsync per document, not per entity), and
//! it is what ensures the facade never needs to acquire a second lock to serve one
//! document — see Design Decision D3 in `plans/2026-07-28-phase1-store-layer.md`.

use async_trait::async_trait;
use std::sync::Mutex;

use crate::audit::entry::{AuditEntry, AuditEntryInput};
use crate::audit::error::AuditError;
use crate::audit::segment::verify_seal_chain;
use crate::audit::segment::{NodeId, Segment, SegmentSeal};

// ── AuditStore trait ──────────────────────────────────────────────────────────

/// A persistence backend for the blake3-chained audit log.
///
/// All methods are `async` so that file and database backends can perform I/O without
/// blocking the async runtime. The in-memory backend happens to do no I/O, but it uses
/// the same signatures so callers never have to care which backend is present.
///
/// Implementations must be `Send + Sync` because the store lives behind an `Arc` shared
/// across tasks and threads. Every method takes `&self` — interior mutability (via
/// `Mutex` or equivalent) is the implementation's responsibility, not the caller's.
///
/// # Object safety
///
/// `Arc<dyn AuditStore>` is constructible. This was verified by a compile-time test
/// (`should_construct_arc_dyn_audit_store`) before any backend was written. If you add a
/// method with a type parameter or a return type that names `Self`, you will break this
/// property and every call site that stores the trait object.
#[async_trait]
pub trait AuditStore: Send + Sync {
    /// Mint and record a document's worth of entries in one call.
    ///
    /// Batched deliberately: it is the transaction boundary a database backend needs,
    /// it is one fsync per document rather than per entity, and it leaves no room for
    /// the two-acquisition shape of §8 gap 5.
    async fn append(&self, inputs: Vec<AuditEntryInput>) -> Result<Vec<AuditEntry>, AuditError>;

    /// Entries in the segment currently open on this writer.
    ///
    /// Returns only the open segment's entries. Sealed segments are accessible via
    /// [`seals`](Self::seals), which carries the entry count for each, or by
    /// re-reading a file backend's JSONL files. Keeping the open segment separate
    /// makes the common case (recent activity) cheap without loading all history.
    async fn entries(&self) -> Result<Vec<AuditEntry>, AuditError>;

    /// The current head of this store's hash chain.
    ///
    /// Cheap by design: it reads one field rather than materialising the segment, because
    /// every content-bearing API response carries it. A client that records the tip
    /// alongside a result can later prove which chain state produced that output, which
    /// is the evidence a DORA audit asks for.
    ///
    /// # Why this is not simply the open segment's tip
    ///
    /// Each segment's entry chain restarts at [`GENESIS_HASH`]; continuity across
    /// segments is carried by the *seal* chain, not the entry chain. So an open segment
    /// with nothing in it yet — the state right after a rotation, or after recovery
    /// sealed an orphan — reports genesis even though the store holds history. Handing
    /// that back would tell a client "nothing has ever been recorded" moments after its
    /// own document was, and two results either side of a rotation would carry the same
    /// tip. The head therefore falls through to the newest seal when the open segment is
    /// empty.
    ///
    /// A genuinely empty store returns [`GENESIS_HASH`], not an error: "nothing recorded
    /// yet" is a legitimate chain state.
    ///
    /// [`GENESIS_HASH`]: crate::audit::GENESIS_HASH
    async fn tip(&self) -> Result<String, AuditError>;

    /// Every seal this store can see, oldest first.
    ///
    /// The seals form a hash chain. Pass the slice to [`verify_seal_chain`] to confirm
    /// no seal was tampered with or deleted since it was recorded.
    async fn seals(&self) -> Result<Vec<SegmentSeal>, AuditError>;

    /// Verify the open segment, every sealed segment, and the links between them.
    ///
    /// A store with no sealed segments and an empty open segment trivially verifies.
    /// A store whose seal chain is broken — because a segment was deleted or a field
    /// was altered — returns an error naming the segment where verification failed.
    async fn verify(&self) -> Result<(), AuditError>;

    /// Seal the open segment and open a successor linked to it.
    ///
    /// Returns the seal that was just created. The successor inherits the seal hash as
    /// its `prev_seal_hash`, so the chain remains unbroken across the rotation.
    ///
    /// Rotation is how a long-running process limits segment size: seal the current run
    /// and keep writing into a new one, without interrupting verification of the whole.
    async fn rotate(&self) -> Result<SegmentSeal, AuditError>;

    /// Seal without opening a successor. Idempotent.
    ///
    /// Calling `close` a second time returns the same seal without creating a new,
    /// empty segment. This is important for shutdown logic that may execute multiple
    /// times (e.g., a signal handler racing a normal exit path).
    async fn close(&self) -> Result<SegmentSeal, AuditError>;
}

// ── InMemoryAuditStore ────────────────────────────────────────────────────────

/// An in-memory implementation of [`AuditStore`].
///
/// State is discarded on drop. This is the right default for testing and for short-lived
/// CLI invocations where durability is not required. For a process that must survive a
/// restart with its audit record intact, use the file backend from Task 3 instead.
///
/// # Locking discipline
///
/// Each method acquires exactly one lock, does all of its work, then drops the guard
/// before returning — no guard crosses an `.await` point. The in-memory backend does no
/// I/O, so there is nothing to await while holding a guard. The file backend in Task 3
/// has to be more careful: it must build the byte buffer under the lock, drop the guard,
/// then `spawn_blocking` the write. A guard held across an `await` makes the future
/// `!Send`, which will not compile behind `Arc<dyn AuditStore>`.
#[derive(Debug)]
pub struct InMemoryAuditStore {
    state: Mutex<State>,
    node_id: NodeId,
    config_hash: String,
}

/// Everything the store mutates, behind **one** lock.
///
/// The three fields were originally three separate mutexes. That is what made `close`
/// racy: it checked `closed_seal`, released, then took `open`, so two concurrent callers
/// could both see "not yet closed" and the loser would find the segment already taken.
/// `close` is documented as safe for a signal handler racing a normal exit, which is
/// exactly that scenario. One lock covering the whole transition removes the window
/// rather than narrowing it, and it makes lock-ordering deadlocks unrepresentable.
///
/// `pub(crate)` (rather than private) so [`super::store_idb::IndexedDbAuditStore`] (Track
/// L5, wasm32-only) can reuse the exact same in-memory bookkeeping and add a persistence
/// step around it, instead of maintaining a second copy of this transition logic.
#[derive(Debug)]
pub(crate) struct State {
    /// The open segment, or `None` once the store is closed.
    pub(crate) open: Option<Segment>,
    /// Sealed segments with their entries, oldest first. The entries are kept because
    /// `verify` has to re-run each sealed segment's internal chain check.
    pub(crate) sealed: Vec<(SegmentSeal, Vec<AuditEntry>)>,
    /// The seal produced by `close`, cached so a second `close` returns the same value
    /// instead of sealing a fresh empty segment.
    pub(crate) closed_seal: Option<SegmentSeal>,
}

impl InMemoryAuditStore {
    /// Create a store whose node identity is derived from the current hostname and pid.
    ///
    /// Use [`with_node_id`](Self::with_node_id) instead if you need a deterministic
    /// identity — in tests, or when two processes share a store directory.
    pub fn new(config_hash: impl Into<String>) -> Self {
        Self::with_node_id(NodeId::default(), config_hash)
    }

    /// Create a store with an explicit node identity.
    ///
    /// `node_id` should be stable across restarts if the store will be recovered by a
    /// file backend, and unique across concurrent writers sharing the same backing store.
    pub fn with_node_id(node_id: NodeId, config_hash: impl Into<String>) -> Self {
        let config_hash = config_hash.into();
        let segment = Segment::open(node_id.clone(), config_hash.clone(), None);
        Self {
            state: Mutex::new(State {
                open: Some(segment),
                sealed: Vec::new(),
                closed_seal: None,
            }),
            node_id,
            config_hash,
        }
    }

    /// Take the state lock, recovering from poisoning.
    ///
    /// A panic cannot leave this state half-written — every mutation below completes
    /// within a single statement sequence holding the guard — so refusing to serve later
    /// callers would discard a usable audit chain for no gain. This mirrors the same
    /// decision already taken in `facade.rs`.
    fn state(&self) -> std::sync::MutexGuard<'_, State> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

#[async_trait]
impl AuditStore for InMemoryAuditStore {
    async fn append(&self, inputs: Vec<AuditEntryInput>) -> Result<Vec<AuditEntry>, AuditError> {
        // One acquisition for the whole batch. That is the entire point of the batch
        // signature: one lock per document, not per entity. The guard is dropped at the
        // end of this block and never crosses an `.await`.
        let mut state = self.state();

        let segment = state.open.as_mut().ok_or(AuditError::StoreClosed {
            operation: "append",
        })?;

        Ok(inputs
            .into_iter()
            .map(|input| segment.push(input).clone())
            .collect())
    }

    async fn entries(&self) -> Result<Vec<AuditEntry>, AuditError> {
        let state = self.state();
        Ok(state
            .open
            .as_ref()
            .map(|segment| segment.entries().to_vec())
            .unwrap_or_default())
    }

    async fn tip(&self) -> Result<String, AuditError> {
        let state = self.state();
        match state.open.as_ref() {
            Some(segment) if !segment.is_empty() => Ok(segment.tip().to_owned()),
            // Empty open segment, or no open segment at all: the newest seal is the head.
            _ => Ok(state
                .sealed
                .last()
                .map(|(seal, _)| seal.sealed_tip.clone())
                .unwrap_or_else(|| crate::audit::GENESIS_HASH.to_owned())),
        }
    }

    async fn seals(&self) -> Result<Vec<SegmentSeal>, AuditError> {
        let state = self.state();
        Ok(state.sealed.iter().map(|(seal, _)| seal.clone()).collect())
    }

    async fn verify(&self) -> Result<(), AuditError> {
        // Snapshot under the lock, then verify without holding it. Verification is O(n)
        // over every entry ever written; blocking all appends for its duration would make
        // a routine integrity check into an outage.
        let (sealed, open_entries, open_tip) = {
            let state = self.state();
            let (entries, tip) = match state.open.as_ref() {
                Some(segment) => (segment.entries().to_vec(), segment.tip().to_owned()),
                None => (Vec::new(), crate::audit::GENESIS_HASH.to_owned()),
            };
            (state.sealed.clone(), entries, tip)
        };

        // Seal chain first. Every later check reads fields off a seal — `config_hash`,
        // `sealed_tip`, `entry_count` — and those fields are only trustworthy once the
        // seal hash covering them has been confirmed. Checking them in the other order
        // would mean validating entries against numbers an attacker chose.
        let seals: Vec<SegmentSeal> = sealed.iter().map(|(seal, _)| seal.clone()).collect();
        verify_seal_chain(&seals)?;

        for (seal, entries) in &sealed {
            verify_sealed_entries(entries, seal)?;
        }

        verify_open_entries(&open_entries, &open_tip, &self.config_hash)
    }

    async fn rotate(&self) -> Result<SegmentSeal, AuditError> {
        let mut state = self.state();

        let old = state.open.take().ok_or(AuditError::StoreClosed {
            operation: "rotate",
        })?;

        let entries = old.entries().to_vec();
        let seal = old.seal();

        // The successor is opened inside the lock on purpose. Between sealing and
        // reopening, the store has no open segment; releasing the lock in that window
        // would let a concurrent `append` see a closed store that is only mid-rotation.
        state.open = Some(Segment::open(
            self.node_id.clone(),
            self.config_hash.clone(),
            Some(seal.seal_hash.clone()),
        ));
        state.sealed.push((seal.clone(), entries));

        Ok(seal)
    }

    async fn close(&self) -> Result<SegmentSeal, AuditError> {
        let mut state = self.state();

        // Idempotent, and idempotent under concurrency: the check and the seal happen
        // under one acquisition, so a second caller either sees the cached seal or blocks
        // until the first has installed it. It can never observe the gap between them.
        if let Some(seal) = &state.closed_seal {
            return Ok(seal.clone());
        }

        let segment = state
            .open
            .take()
            .ok_or(AuditError::StoreClosed { operation: "close" })?;

        let entries = segment.entries().to_vec();
        let seal = segment.seal();

        state.sealed.push((seal.clone(), entries));
        state.closed_seal = Some(seal.clone());

        Ok(seal)
    }
}

// ── internal helpers ──────────────────────────────────────────────────────────

/// Replay a sealed segment's entries from genesis and confirm they match the seal.
///
/// Shared by [`InMemoryAuditStore`] and [`super::store_file::FileAuditStore`]. A second
/// copy of this logic that drifted from the first is exactly the failure mode the crate
/// cannot afford — one sealed verification path, used by both backends.
///
/// This is what makes `verify()` honest about history: it does not just check the seal
/// chain links, it also confirms that each segment's own entry chain is internally
/// consistent, holds the number of entries the seal recorded, and ends at the hash the
/// seal records.
///
/// The seal is the authority for both `config_hash` and `entry_count`. Its own hash
/// covers every one of its fields and has already been confirmed by [`verify_seal_chain`]
/// before this function runs, so reading them here is not circular.
pub(crate) fn verify_sealed_entries(
    entries: &[AuditEntry],
    seal: &SegmentSeal,
) -> Result<(), AuditError> {
    // Count first. A missing entry produces a wrong tip too, but "holds 37 entries, seal
    // records 40" tells the operator what happened; an opaque hash mismatch does not.
    if entries.len() as u64 != seal.entry_count {
        return Err(AuditError::SegmentEntryCount {
            segment_id: seal.segment_id.clone(),
            expected: seal.entry_count,
            actual: entries.len() as u64,
        });
    }

    let mut chain = crate::audit::chain::AuditChain::new(seal.config_hash.clone());

    for entry in entries {
        chain.append(entry.clone())?;
    }

    chain.verify()?;

    if chain.tip() != seal.sealed_tip {
        return Err(AuditError::ChainIntegrity {
            index: entries.len() as u64,
            expected: seal.sealed_tip.clone(),
            actual: chain.tip().to_owned(),
        });
    }

    Ok(())
}

/// Replay the open segment's entries from genesis and confirm they reach `expected_tip`.
///
/// `config_hash` must come from the **store**, not from the entries. Taking it from
/// `entries[0]` would be circular: an attacker who rewrites every entry under a single
/// consistent config hash of their choosing would replay cleanly. The store's own value
/// is the independent witness that catches that substitution.
///
/// The open segment has no seal to cross-check against, so the tip is checked against the
/// live segment's recorded tip rather than an immutable record. That is weaker than the
/// sealed case by construction — an open segment is still being written.
///
/// Shared by [`InMemoryAuditStore`] and [`super::store_file::FileAuditStore`]. Taking
/// `config_hash` as an explicit parameter rather than reading `entries[0]` is deliberate
/// and must not be changed — see the circular-verification argument above.
pub(crate) fn verify_open_entries(
    entries: &[AuditEntry],
    expected_tip: &str,
    config_hash: &str,
) -> Result<(), AuditError> {
    if entries.is_empty() {
        return Ok(());
    }

    let mut chain = crate::audit::chain::AuditChain::new(config_hash.to_owned());

    for entry in entries {
        chain.append(entry.clone())?;
    }

    chain.verify()?;

    if chain.tip() != expected_tip {
        return Err(AuditError::ChainIntegrity {
            index: entries.len() as u64,
            expected: expected_tip.to_owned(),
            actual: chain.tip().to_owned(),
        });
    }

    Ok(())
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audit::entry::{EntitySource, RedactionAction};
    use std::sync::Arc;

    // ── helpers ──────────────────────────────────────────────────────────────

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
            config_hash: "test-config".into(),
            principal: None,
        }
    }

    fn make_store() -> InMemoryAuditStore {
        InMemoryAuditStore::with_node_id(NodeId::new("test-node"), "test-config")
    }

    /// The tip is the evidence an API response hands back with every result, so it has to
    /// agree with the chain it claims to describe: genesis before anything is written,
    /// and the newest entry's `chain_hash` afterwards. A tip that lagged behind the chain
    /// would let a client "prove" a result against a state that never produced it.
    #[tokio::test]
    async fn should_report_genesis_then_track_the_newest_entry() {
        let store = make_store();
        assert_eq!(store.tip().await.unwrap(), crate::audit::GENESIS_HASH);

        let first = store.append(vec![make_input("a")]).await.unwrap();
        assert_eq!(store.tip().await.unwrap(), first[0].chain_hash);

        let second = store.append(vec![make_input("b")]).await.unwrap();
        assert_eq!(store.tip().await.unwrap(), second[0].chain_hash);
        assert_ne!(first[0].chain_hash, second[0].chain_hash);
    }

    /// Closing seals the open segment. The chain did not disappear, so the tip must be
    /// the seal's, not a reset to genesis — otherwise a result issued just before
    /// shutdown could not be tied to the record that proves it.
    #[tokio::test]
    async fn should_report_the_seal_tip_after_close() {
        let store = make_store();
        let entries = store.append(vec![make_input("a")]).await.unwrap();
        let seal = store.close().await.unwrap();

        assert_eq!(seal.sealed_tip, entries[0].chain_hash);
        assert_eq!(store.tip().await.unwrap(), seal.sealed_tip);
    }

    /// Rotation opens an empty successor whose own entry chain starts at genesis. The
    /// reported head must not fall back to genesis with it: a caller that processed a
    /// document before the rotation and another that queried after would otherwise be
    /// handed the same "empty chain" evidence.
    #[tokio::test]
    async fn should_not_reset_to_genesis_when_rotation_opens_an_empty_segment() {
        let store = make_store();
        let entries = store.append(vec![make_input("a")]).await.unwrap();
        let seal = store.rotate().await.unwrap();

        assert_eq!(store.tip().await.unwrap(), seal.sealed_tip);
        assert_ne!(store.tip().await.unwrap(), crate::audit::GENESIS_HASH);

        // Once the successor has entries of its own, it takes over again.
        let after = store.append(vec![make_input("b")]).await.unwrap();
        assert_eq!(store.tip().await.unwrap(), after[0].chain_hash);
        assert_ne!(after[0].chain_hash, entries[0].chain_hash);
    }

    // ── Step 1: object-safety check ──────────────────────────────────────────

    /// Confirm that `Arc<dyn AuditStore>` is constructible.
    ///
    /// This is the pre-condition for the whole trait design. If the trait is not object-
    /// safe — because a method is generic, or returns `Self`, or uses an associated type
    /// the erased trait cannot carry — this test fails to compile. Writing it before any
    /// backend exists forces the error to appear at the right level: the trait, not a
    /// specific implementation.
    #[tokio::test]
    async fn should_construct_arc_dyn_audit_store() {
        let store = InMemoryAuditStore::with_node_id(NodeId::new("test-node"), "cfg");
        let _: Arc<dyn AuditStore> = Arc::new(store);
        // If this line compiles, the trait is object-safe.
    }

    // ── Step 5 tests ─────────────────────────────────────────────────────────

    #[tokio::test]
    async fn should_return_one_entry_per_input_in_order() {
        let store = make_store();
        let inputs = vec![make_input("e1"), make_input("e2"), make_input("e3")];
        let entries = store.append(inputs).await.expect("append must succeed");
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0].id, "e1");
        assert_eq!(entries[1].id, "e2");
        assert_eq!(entries[2].id, "e3");
    }

    #[tokio::test]
    async fn should_chain_entries_across_two_append_calls() {
        let store = make_store();
        store
            .append(vec![make_input("a1"), make_input("a2")])
            .await
            .expect("first append");
        store
            .append(vec![make_input("b1"), make_input("b2")])
            .await
            .expect("second append");
        let entries = store.entries().await.expect("entries");
        assert_eq!(entries.len(), 4);
        // The chain must verify internally — entries from both appends are chained.
        store
            .verify()
            .await
            .expect("chain must verify after two appends");
    }

    #[tokio::test]
    async fn should_verify_after_a_rotation() {
        let store = make_store();
        store
            .append(vec![make_input("pre-rotate")])
            .await
            .expect("append before rotate");
        store.rotate().await.expect("rotate");
        store
            .append(vec![make_input("post-rotate")])
            .await
            .expect("append after rotate");
        store.verify().await.expect("verify after rotation");
    }

    #[tokio::test]
    async fn should_link_the_new_segment_to_the_sealed_one_on_rotate() {
        let store = make_store();
        store
            .append(vec![make_input("first")])
            .await
            .expect("append");
        let seal = store.rotate().await.expect("rotate");
        // After rotation the open segment's entries come from the new segment, not the sealed one.
        let open_entries = store.entries().await.expect("entries");
        assert_eq!(open_entries.len(), 0, "new segment should start empty");
        // The sealed list should now contain the one we just sealed.
        let seals = store.seals().await.expect("seals");
        assert_eq!(seals.len(), 1);
        assert_eq!(seals[0].seal_hash, seal.seal_hash);
        // The next segment we open should record the seal hash as its predecessor.
        store
            .append(vec![make_input("second")])
            .await
            .expect("append after rotate");
        let seal2 = store.rotate().await.expect("second rotate");
        assert_eq!(
            seal2.prev_seal_hash.as_deref(),
            Some(seal.seal_hash.as_str()),
            "successor seal must point at the predecessor"
        );
    }

    #[tokio::test]
    async fn should_report_entries_from_the_open_segment_only() {
        let store = make_store();
        store
            .append(vec![make_input("sealed-1")])
            .await
            .expect("append");
        store.rotate().await.expect("rotate");
        store
            .append(vec![make_input("open-1"), make_input("open-2")])
            .await
            .expect("append to new segment");
        let entries = store.entries().await.expect("entries");
        // Only the open segment's entries.
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].id, "open-1");
        assert_eq!(entries[1].id, "open-2");
    }

    #[tokio::test]
    async fn should_be_idempotent_when_closed_twice() {
        let store = make_store();
        store.append(vec![make_input("x")]).await.expect("append");
        let seal1 = store.close().await.expect("first close");
        let seal2 = store.close().await.expect("second close");
        assert_eq!(
            seal1.seal_hash, seal2.seal_hash,
            "both close calls must return the same seal"
        );
    }

    /// The open segment's config hash must come from the store, never from the entries.
    ///
    /// This pins the third parameter of `verify_open_entries`. An earlier version read
    /// `entries[0].config_hash`, which is circular: an attacker who rewrites every entry
    /// under one config hash of their own choosing produces a set that is internally
    /// consistent and replays cleanly. Verifying against the store's independently held
    /// value is what rejects the substitution.
    #[test]
    fn should_reject_open_entries_minted_under_a_substituted_config() {
        let mut foreign = crate::audit::chain::AuditChain::new("attacker-config");
        foreign.push(make_input("s1"));
        foreign.push(make_input("s2"));

        // Internally consistent — the attacker re-minted the whole run, so this passes.
        foreign
            .verify()
            .expect("the substituted chain is self-consistent");

        let err = verify_open_entries(foreign.entries(), foreign.tip(), "test-config")
            .expect_err("a foreign config hash must be rejected");
        match err {
            AuditError::ConfigMismatch { expected, actual } => {
                assert_eq!(expected, "test-config");
                assert_eq!(actual, "attacker-config");
            }
            other => panic!("expected ConfigMismatch, got {other:?}"),
        }
    }

    /// The suite must be able to see `verify()` fail, not only succeed.
    ///
    /// Every other test here asserts `verify().is_ok()`. A `verify` that returned `Ok(())`
    /// unconditionally would pass all of them. This is the one test that would catch that,
    /// and it is the reason it exists: tamper with a byte of a sealed entry and require the
    /// error to name the sealed segment's chain, not the seal chain.
    #[tokio::test]
    async fn should_detect_a_tampered_entry_in_a_sealed_segment() {
        let store = make_store();
        store
            .append(vec![make_input("t1"), make_input("t2")])
            .await
            .expect("append");
        store.rotate().await.expect("rotate seals the segment");
        store.verify().await.expect("clean chain verifies");

        // Alter a field that is committed into the chain hash, leaving the recorded
        // `chain_hash` untouched — exactly what an editor of the log file would produce.
        {
            let mut state = store.state();
            state.sealed[0].1[1].category = "CreditCard".into();
        }

        let err = store
            .verify()
            .await
            .expect_err("a tampered sealed entry must fail verification");
        assert!(
            matches!(err, AuditError::ChainIntegrity { index: 1, .. }),
            "expected ChainIntegrity at index 1, got {err:?}"
        );
    }

    /// Removing entries from a sealed segment is reported as a count error, not a hash one.
    ///
    /// Deletion also breaks the tip, so a bare hash check would already fail. The point of
    /// the separate error is that "holds 1 entry, seal records 2" is actionable and an
    /// opaque hash mismatch is not — see Design Decision D2.
    #[tokio::test]
    async fn should_report_a_missing_entry_as_a_count_mismatch() {
        let store = make_store();
        store
            .append(vec![make_input("c1"), make_input("c2")])
            .await
            .expect("append");
        store.rotate().await.expect("rotate");

        {
            let mut state = store.state();
            state.sealed[0].1.pop();
        }

        let err = store
            .verify()
            .await
            .expect_err("a truncated sealed segment must fail verification");
        match err {
            AuditError::SegmentEntryCount {
                expected, actual, ..
            } => {
                assert_eq!(expected, 2);
                assert_eq!(actual, 1);
            }
            other => panic!("expected SegmentEntryCount, got {other:?}"),
        }
    }

    /// `close` is documented as safe for a signal handler racing a normal exit path.
    ///
    /// The sequential idempotence test above cannot show that: the first call has fully
    /// returned before the second begins. Only concurrent callers exercise the window
    /// between reading `closed_seal` and installing it, which is why the three fields of
    /// `State` share one lock.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn should_be_idempotent_when_closed_concurrently() {
        let store = Arc::new(make_store());
        store.append(vec![make_input("x")]).await.expect("append");

        const CLOSERS: usize = 8;
        let mut handles = Vec::with_capacity(CLOSERS);
        for _ in 0..CLOSERS {
            let store_clone = Arc::clone(&store);
            handles.push(tokio::spawn(async move { store_clone.close().await }));
        }

        let mut hashes = Vec::with_capacity(CLOSERS);
        for handle in handles {
            let seal = handle
                .await
                .expect("closer must not panic")
                .expect("every concurrent close must succeed");
            hashes.push(seal.seal_hash);
        }

        assert!(
            hashes.windows(2).all(|pair| pair[0] == pair[1]),
            "all concurrent closes must return one seal, got {hashes:?}"
        );
        // Exactly one segment was sealed — no closer minted a second, empty one.
        assert_eq!(store.seals().await.expect("seals").len(), 1);
    }

    /// Using a closed store is a lifecycle mistake, not evidence of tampering.
    ///
    /// It must not surface as [`AuditError::ChainIntegrity`]: a caller matching on that
    /// variant to raise a tamper alarm would fire on a shutdown-ordering bug.
    #[tokio::test]
    async fn should_refuse_to_append_after_close() {
        let store = make_store();
        store.close().await.expect("close");

        let err = store
            .append(vec![make_input("late")])
            .await
            .expect_err("append after close must fail");
        assert!(
            matches!(
                err,
                AuditError::StoreClosed {
                    operation: "append"
                }
            ),
            "expected StoreClosed{{ operation: \"append\" }}, got {err:?}"
        );

        let err = store
            .rotate()
            .await
            .expect_err("rotate after close must fail");
        assert!(
            matches!(
                err,
                AuditError::StoreClosed {
                    operation: "rotate"
                }
            ),
            "expected StoreClosed{{ operation: \"rotate\" }}, got {err:?}"
        );
    }

    #[tokio::test]
    async fn should_serialise_concurrent_appends_without_breaking_the_chain() {
        let store = Arc::new(InMemoryAuditStore::with_node_id(
            NodeId::new("concurrent-test"),
            "test-config",
        ));

        const TASKS: usize = 8;
        const ENTRIES_PER_TASK: usize = 10;

        let mut handles = Vec::with_capacity(TASKS);
        for task in 0..TASKS {
            let store_clone = Arc::clone(&store);
            let handle = tokio::spawn(async move {
                let inputs: Vec<AuditEntryInput> = (0..ENTRIES_PER_TASK)
                    .map(|i| make_input(&format!("task-{task}-entry-{i}")))
                    .collect();
                store_clone
                    .append(inputs)
                    .await
                    .expect("concurrent append must succeed")
            });
            handles.push(handle);
        }

        for handle in handles {
            handle.await.expect("task must not panic");
        }

        // Every append serialised through the mutex, so the full chain must be valid.
        store
            .verify()
            .await
            .expect("chain must verify after concurrent appends");

        let all_entries = store.entries().await.expect("entries");
        assert_eq!(all_entries.len(), TASKS * ENTRIES_PER_TASK);
    }
}
