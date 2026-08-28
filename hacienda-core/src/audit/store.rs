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
use std::collections::HashMap;
use std::sync::Mutex;

use crate::audit::cursor::{page_from, AuditCursor, AuditPage};
use crate::audit::entry::{AuditEntry, AuditEntryInput};
use crate::audit::error::AuditError;
use crate::audit::segment::verify_seal_chain;
use crate::audit::segment::{NodeId, Segment, SegmentSeal};
use crate::tenancy::TenantId;

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
/// # Tenant scoping (S1b)
///
/// Every method takes `&TenantId` and operates on **that tenant's own chain only** — one
/// hash chain per `(NodeId, TenantId)`, not one chain per node filtered on read. See
/// `superpowers/specs/2026-08-14-S1b-tenant-scoped-audit-review-job-document-stores.md`
/// §1.2 for why a shared-chain-filtered-on-read design was rejected, and
/// `hacienda-core/src/tenancy.rs`'s module doc (decision D-S1-1) for why the tenant is a
/// parameter on every call rather than baked into the store at construction time: a
/// parameter fails to compile at every call site that forgets it, where a
/// one-store-per-tenant factory would let the omission (a store accidentally shared
/// across tenants) go unnoticed.
///
/// # Object safety
///
/// `Arc<dyn AuditStore>` is constructible. This was verified by a compile-time test
/// (`should_construct_arc_dyn_audit_store`) before any backend was written. If you add a
/// method with a type parameter or a return type that names `Self`, you will break this
/// property and every call site that stores the trait object.
#[async_trait]
pub trait AuditStore: Send + Sync {
    /// Mint and record a document's worth of entries in one call, into `tenant`'s chain.
    ///
    /// Batched deliberately: it is the transaction boundary a database backend needs,
    /// it is one fsync per document rather than per entity, and it leaves no room for
    /// the two-acquisition shape of §8 gap 5.
    async fn append(
        &self,
        tenant: &TenantId,
        inputs: Vec<AuditEntryInput>,
    ) -> Result<Vec<AuditEntry>, AuditError>;

    /// Entries in the segment currently open on this writer, for `tenant`.
    ///
    /// Returns only the open segment's entries. Sealed segments are accessible via
    /// [`seals`](Self::seals), which carries the entry count for each, or by
    /// re-reading a file backend's JSONL files. Keeping the open segment separate
    /// makes the common case (recent activity) cheap without loading all history.
    async fn entries(&self, tenant: &TenantId) -> Result<Vec<AuditEntry>, AuditError>;

    /// One page of the entries of every sealed segment, then of the open one, oldest
    /// first — **for this node**.
    ///
    /// This is the method that answers "what has this store recorded"; [`entries`] answers
    /// only "what has it recorded since the last rotation". An API that offered the latter
    /// under the former's name would let an auditor conclude that events which exist never
    /// happened, which is a worse outcome than offering no history at all.
    ///
    /// # Scope: this node, not this deployment
    ///
    /// Segments are per-writer and there is no total order between writers — a
    /// [`FileAuditStore`](crate::audit::FileAuditStore) sees only its own `node_id`
    /// directory. What comes back is therefore *this node's* history. Anything presenting
    /// it as the deployment's history repeats, one level up, the exact mistake this method
    /// exists to close.
    ///
    /// # Paging
    ///
    /// `after` is an opaque [`AuditCursor`] this method previously returned — never an
    /// offset, never an entry id. A cursor that cannot be resolved is an error
    /// ([`AuditError::UnresolvableCursor`]), never a silent restart from the beginning.
    /// `limit` caps the page; `0` yields an empty page. See [`AuditPage::next`] for when
    /// paging stops.
    ///
    /// Entries appended while a caller pages appear on a later page rather than shifting
    /// the ones already served: the chain grows at the end, and a cursor names a position
    /// that growth does not move.
    ///
    /// [`entries`]: Self::entries
    async fn history(
        &self,
        tenant: &TenantId,
        after: Option<&AuditCursor>,
        limit: usize,
    ) -> Result<AuditPage, AuditError>;

    /// The current head of `tenant`'s hash chain.
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
    async fn tip(&self, tenant: &TenantId) -> Result<String, AuditError>;

    /// Every seal `tenant` can see, oldest first.
    ///
    /// The seals form a hash chain. Pass the slice to [`verify_seal_chain`] to confirm
    /// no seal was tampered with or deleted since it was recorded.
    async fn seals(&self, tenant: &TenantId) -> Result<Vec<SegmentSeal>, AuditError>;

    /// Verify `tenant`'s open segment, every sealed segment, and the links between them.
    ///
    /// A store with no sealed segments and an empty open segment trivially verifies.
    /// A store whose seal chain is broken — because a segment was deleted or a field
    /// was altered — returns an error naming the segment where verification failed.
    async fn verify(&self, tenant: &TenantId) -> Result<(), AuditError>;

    /// Seal `tenant`'s open segment and open a successor linked to it.
    ///
    /// Returns the seal that was just created. The successor inherits the seal hash as
    /// its `prev_seal_hash`, so the chain remains unbroken across the rotation.
    ///
    /// Rotation is how a long-running process limits segment size: seal the current run
    /// and keep writing into a new one, without interrupting verification of the whole.
    async fn rotate(&self, tenant: &TenantId) -> Result<SegmentSeal, AuditError>;

    /// Seal `tenant`'s open segment without opening a successor. Idempotent.
    ///
    /// Calling `close` a second time returns the same seal without creating a new,
    /// empty segment. This is important for shutdown logic that may execute multiple
    /// times (e.g., a signal handler racing a normal exit path).
    async fn close(&self, tenant: &TenantId) -> Result<SegmentSeal, AuditError>;
}

// ── InMemoryAuditStore ────────────────────────────────────────────────────────

/// An in-memory implementation of [`AuditStore`].
///
/// State is discarded on drop. This is the right default for testing and for short-lived
/// CLI invocations where durability is not required. For a process that must survive a
/// restart with its audit record intact, use the file backend from Task 3 instead.
///
/// One [`State`] per tenant, lazily created on that tenant's first call — not one
/// `InMemoryAuditStore` per tenant (see the trait doc's "Tenant scoping" section for why).
/// `node_id`/`config_hash` are shared: this is one writer node serving many tenants, each
/// with its own independent chain.
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
/// InMemoryAuditStore struct
pub struct InMemoryAuditStore {
    state: Mutex<HashMap<TenantId, State>>,
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
        Self {
            state: Mutex::new(HashMap::new()),
            node_id,
            config_hash: config_hash.into(),
        }
    }

    /// Take the state lock, recovering from poisoning.
    ///
    /// A panic cannot leave this state half-written — every mutation below completes
    /// within a single statement sequence holding the guard — so refusing to serve later
    /// callers would discard a usable audit chain for no gain. This mirrors the same
    /// decision already taken in `facade.rs`.
    fn state(&self) -> std::sync::MutexGuard<'_, HashMap<TenantId, State>> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// A fresh, empty, open segment for a tenant touching this store for the first time.
    fn fresh_state(node_id: &NodeId, config_hash: &str) -> State {
        State {
            open: Some(Segment::open(node_id.clone(), config_hash.to_owned(), None)),
            sealed: Vec::new(),
            closed_seal: None,
        }
    }

    /// Get or lazily create `tenant`'s [`State`] within an already-locked map.
    ///
    /// Lazy per-tenant creation on first touch, rather than requiring an explicit
    /// "provision this tenant" call, mirrors how a freshly constructed store already
    /// starts with an implicit open segment for its one (pre-S1b) chain.
    fn tenant_state<'a>(
        map: &'a mut HashMap<TenantId, State>,
        tenant: &TenantId,
        node_id: &NodeId,
        config_hash: &str,
    ) -> &'a mut State {
        map.entry(tenant.clone())
            .or_insert_with(|| Self::fresh_state(node_id, config_hash))
    }
}

#[async_trait]
impl AuditStore for InMemoryAuditStore {
    async fn append(
        &self,
        tenant: &TenantId,
        inputs: Vec<AuditEntryInput>,
    ) -> Result<Vec<AuditEntry>, AuditError> {
        // One acquisition for the whole batch. That is the entire point of the batch
        // signature: one lock per document, not per entity. The guard is dropped at the
        // end of this block and never crosses an `.await`.
        let mut map = self.state();
        let state = Self::tenant_state(&mut map, tenant, &self.node_id, &self.config_hash);

        let segment = state.open.as_mut().ok_or(AuditError::StoreClosed {
            operation: "append",
        })?;

        Ok(inputs
            .into_iter()
            .map(|input| segment.push(input).clone())
            .collect())
    }

    async fn entries(&self, tenant: &TenantId) -> Result<Vec<AuditEntry>, AuditError> {
        let mut map = self.state();
        let state = Self::tenant_state(&mut map, tenant, &self.node_id, &self.config_hash);
        Ok(state
            .open
            .as_ref()
            .map(|segment| segment.entries().to_vec())
            .unwrap_or_default())
    }

    /// The whole history is already in memory, so the guard is simply held for the
    /// duration — the page is assembled from borrowed state and nothing is awaited inside.
    /// Snapshotting first, as `verify` does, would copy every entry ever written just to
    /// hand back at most `limit` of them.
    ///
    /// Sealed extents are taken from each **seal**'s `entry_count`, not from the length of
    /// the vector beside it. The seal is the immutable record; disagreement between the
    /// two is exactly what [`AuditError::SegmentEntryCount`] reports, and reading the
    /// length would define that discrepancy out of existence.
    async fn history(
        &self,
        tenant: &TenantId,
        after: Option<&AuditCursor>,
        limit: usize,
    ) -> Result<AuditPage, AuditError> {
        let mut map = self.state();
        let state = Self::tenant_state(&mut map, tenant, &self.node_id, &self.config_hash);

        let mut extents: Vec<(&str, u64)> = state
            .sealed
            .iter()
            .map(|(seal, _)| (seal.segment_id.as_str(), seal.entry_count))
            .collect();
        if let Some(segment) = state.open.as_ref() {
            extents.push((segment.id(), segment.len() as u64));
        }

        page_from(&extents, after, limit, |position| {
            Ok(match state.sealed.get(position) {
                Some((_, entries)) => entries.clone(),
                // Past the sealed run, so this is the open segment.
                None => state
                    .open
                    .as_ref()
                    .map(|segment| segment.entries().to_vec())
                    .unwrap_or_default(),
            })
        })
    }

    async fn tip(&self, tenant: &TenantId) -> Result<String, AuditError> {
        let mut map = self.state();
        let state = Self::tenant_state(&mut map, tenant, &self.node_id, &self.config_hash);
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

    async fn seals(&self, tenant: &TenantId) -> Result<Vec<SegmentSeal>, AuditError> {
        let mut map = self.state();
        let state = Self::tenant_state(&mut map, tenant, &self.node_id, &self.config_hash);
        Ok(state.sealed.iter().map(|(seal, _)| seal.clone()).collect())
    }

    async fn verify(&self, tenant: &TenantId) -> Result<(), AuditError> {
        // Snapshot under the lock, then verify without holding it. Verification is O(n)
        // over every entry ever written; blocking all appends for its duration would make
        // a routine integrity check into an outage.
        let (sealed, open_entries, open_tip) = {
            let mut map = self.state();
            let state = Self::tenant_state(&mut map, tenant, &self.node_id, &self.config_hash);
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

    async fn rotate(&self, tenant: &TenantId) -> Result<SegmentSeal, AuditError> {
        let mut map = self.state();
        let state = Self::tenant_state(&mut map, tenant, &self.node_id, &self.config_hash);

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

    async fn close(&self, tenant: &TenantId) -> Result<SegmentSeal, AuditError> {
        let mut map = self.state();
        let state = Self::tenant_state(&mut map, tenant, &self.node_id, &self.config_hash);

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
            vertical: None,
            model: None,
        }
    }

    fn make_store() -> InMemoryAuditStore {
        InMemoryAuditStore::with_node_id(NodeId::new("test-node"), "test-config")
    }

    /// The tenant used by every test that doesn't itself care about tenant scoping —
    /// keeps the pre-S1b test bodies unchanged apart from threading this through.
    fn t() -> TenantId {
        TenantId::new("tenant-a")
    }

    /// A second, distinct tenant — used only by the cross-tenant isolation tests below.
    fn t2() -> TenantId {
        TenantId::new("tenant-b")
    }

    /// The tip is the evidence an API response hands back with every result, so it has to
    /// agree with the chain it claims to describe: genesis before anything is written,
    /// and the newest entry's `chain_hash` afterwards. A tip that lagged behind the chain
    /// would let a client "prove" a result against a state that never produced it.
    #[tokio::test]
    async fn should_report_genesis_then_track_the_newest_entry() {
        let store = make_store();
        assert_eq!(store.tip(&t()).await.unwrap(), crate::audit::GENESIS_HASH);

        let first = store.append(&t(), vec![make_input("a")]).await.unwrap();
        assert_eq!(store.tip(&t()).await.unwrap(), first[0].chain_hash);

        let second = store.append(&t(), vec![make_input("b")]).await.unwrap();
        assert_eq!(store.tip(&t()).await.unwrap(), second[0].chain_hash);
        assert_ne!(first[0].chain_hash, second[0].chain_hash);
    }

    /// Closing seals the open segment. The chain did not disappear, so the tip must be
    /// the seal's, not a reset to genesis — otherwise a result issued just before
    /// shutdown could not be tied to the record that proves it.
    #[tokio::test]
    async fn should_report_the_seal_tip_after_close() {
        let store = make_store();
        let entries = store.append(&t(), vec![make_input("a")]).await.unwrap();
        let seal = store.close(&t()).await.unwrap();

        assert_eq!(seal.sealed_tip, entries[0].chain_hash);
        assert_eq!(store.tip(&t()).await.unwrap(), seal.sealed_tip);
    }

    /// Rotation opens an empty successor whose own entry chain starts at genesis. The
    /// reported head must not fall back to genesis with it: a caller that processed a
    /// document before the rotation and another that queried after would otherwise be
    /// handed the same "empty chain" evidence.
    #[tokio::test]
    async fn should_not_reset_to_genesis_when_rotation_opens_an_empty_segment() {
        let store = make_store();
        let entries = store.append(&t(), vec![make_input("a")]).await.unwrap();
        let seal = store.rotate(&t()).await.unwrap();

        assert_eq!(store.tip(&t()).await.unwrap(), seal.sealed_tip);
        assert_ne!(store.tip(&t()).await.unwrap(), crate::audit::GENESIS_HASH);

        // Once the successor has entries of its own, it takes over again.
        let after = store.append(&t(), vec![make_input("b")]).await.unwrap();
        assert_eq!(store.tip(&t()).await.unwrap(), after[0].chain_hash);
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
        let entries = store
            .append(&t(), inputs)
            .await
            .expect("append must succeed");
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0].id, "e1");
        assert_eq!(entries[1].id, "e2");
        assert_eq!(entries[2].id, "e3");
    }

    #[tokio::test]
    async fn should_chain_entries_across_two_append_calls() {
        let store = make_store();
        store
            .append(&t(), vec![make_input("a1"), make_input("a2")])
            .await
            .expect("first append");
        store
            .append(&t(), vec![make_input("b1"), make_input("b2")])
            .await
            .expect("second append");
        let entries = store.entries(&t()).await.expect("entries");
        assert_eq!(entries.len(), 4);
        // The chain must verify internally — entries from both appends are chained.
        store
            .verify(&t())
            .await
            .expect("chain must verify after two appends");
    }

    #[tokio::test]
    async fn should_verify_after_a_rotation() {
        let store = make_store();
        store
            .append(&t(), vec![make_input("pre-rotate")])
            .await
            .expect("append before rotate");
        store.rotate(&t()).await.expect("rotate");
        store
            .append(&t(), vec![make_input("post-rotate")])
            .await
            .expect("append after rotate");
        store.verify(&t()).await.expect("verify after rotation");
    }

    #[tokio::test]
    async fn should_link_the_new_segment_to_the_sealed_one_on_rotate() {
        let store = make_store();
        store
            .append(&t(), vec![make_input("first")])
            .await
            .expect("append");
        let seal = store.rotate(&t()).await.expect("rotate");
        // After rotation the open segment's entries come from the new segment, not the sealed one.
        let open_entries = store.entries(&t()).await.expect("entries");
        assert_eq!(open_entries.len(), 0, "new segment should start empty");
        // The sealed list should now contain the one we just sealed.
        let seals = store.seals(&t()).await.expect("seals");
        assert_eq!(seals.len(), 1);
        assert_eq!(seals[0].seal_hash, seal.seal_hash);
        // The next segment we open should record the seal hash as its predecessor.
        store
            .append(&t(), vec![make_input("second")])
            .await
            .expect("append after rotate");
        let seal2 = store.rotate(&t()).await.expect("second rotate");
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
            .append(&t(), vec![make_input("sealed-1")])
            .await
            .expect("append");
        store.rotate(&t()).await.expect("rotate");
        store
            .append(&t(), vec![make_input("open-1"), make_input("open-2")])
            .await
            .expect("append to new segment");
        let entries = store.entries(&t()).await.expect("entries");
        // Only the open segment's entries.
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].id, "open-1");
        assert_eq!(entries[1].id, "open-2");
    }

    /// The in-memory backend is the facade's default, so the gap `history` closes is the
    /// same one here: `entries()` reports the open segment, and after two rotations plus a
    /// close it reports nothing at all while the store still holds the whole run.
    #[tokio::test]
    async fn should_return_every_entry_across_rotations_and_after_close() {
        let store = make_store();

        store
            .append(&t(), vec![make_input("m1"), make_input("m2")])
            .await
            .expect("append");
        store.rotate(&t()).await.expect("rotate 1");
        store
            .append(&t(), vec![make_input("m3")])
            .await
            .expect("append");
        store.rotate(&t()).await.expect("rotate 2");
        store
            .append(&t(), vec![make_input("m4")])
            .await
            .expect("append");

        let expected = vec!["m1", "m2", "m3", "m4"];

        let ids = |page: &AuditPage| -> Vec<String> {
            page.entries.iter().map(|entry| entry.id.clone()).collect()
        };

        let page = store.history(&t(), None, 100).await.expect("history");
        assert_eq!(ids(&page), expected);

        // Paging one at a time crosses every segment boundary with a cursor in hand.
        let mut collected = Vec::new();
        let mut cursor = None;
        loop {
            let page = store
                .history(&t(), cursor.as_ref(), 1)
                .await
                .expect("history");
            if page.entries.is_empty() {
                break;
            }
            collected.extend(ids(&page));
            cursor = page.next;
        }
        assert_eq!(collected, expected);

        store.close(&t()).await.expect("close");
        assert!(
            store.entries(&t()).await.expect("entries").is_empty(),
            "entries() reports the open segment, and close left none"
        );
        let page = store
            .history(&t(), None, 100)
            .await
            .expect("history after close");
        assert_eq!(
            ids(&page),
            expected,
            "closing seals the final segment; it must not disappear from the history"
        );
    }

    /// An unresolvable cursor is an error here too — a silent restart would hand the
    /// caller every entry it already holds a second time, indistinguishable from new
    /// activity.
    #[tokio::test]
    async fn should_reject_a_cursor_this_store_cannot_resolve() {
        let store = make_store();
        store
            .append(&t(), vec![make_input("u1")])
            .await
            .expect("append");

        let foreign = crate::audit::AuditCursor {
            segment_id: "00000000-0000-4000-8000-000000000000".into(),
            index: 0,
        };
        let err = store
            .history(&t(), Some(&foreign), 10)
            .await
            .expect_err("an unknown segment must not be served as a fresh start");
        assert!(
            matches!(err, AuditError::UnresolvableCursor { .. }),
            "expected UnresolvableCursor, got {err:?}"
        );
    }

    #[tokio::test]
    async fn should_be_idempotent_when_closed_twice() {
        let store = make_store();
        store
            .append(&t(), vec![make_input("x")])
            .await
            .expect("append");
        let seal1 = store.close(&t()).await.expect("first close");
        let seal2 = store.close(&t()).await.expect("second close");
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
            .append(&t(), vec![make_input("t1"), make_input("t2")])
            .await
            .expect("append");
        store.rotate(&t()).await.expect("rotate seals the segment");
        store.verify(&t()).await.expect("clean chain verifies");

        // Alter a field that is committed into the chain hash, leaving the recorded
        // `chain_hash` untouched — exactly what an editor of the log file would produce.
        {
            let mut map = store.state();
            let state = map.get_mut(&t()).expect("tenant state must exist");
            state.sealed[0].1[1].category = "CreditCard".into();
        }

        let err = store
            .verify(&t())
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
            .append(&t(), vec![make_input("c1"), make_input("c2")])
            .await
            .expect("append");
        store.rotate(&t()).await.expect("rotate");

        {
            let mut map = store.state();
            let state = map.get_mut(&t()).expect("tenant state must exist");
            state.sealed[0].1.pop();
        }

        let err = store
            .verify(&t())
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
        store
            .append(&t(), vec![make_input("x")])
            .await
            .expect("append");

        const CLOSERS: usize = 8;
        let mut handles = Vec::with_capacity(CLOSERS);
        for _ in 0..CLOSERS {
            let store_clone = Arc::clone(&store);
            handles.push(tokio::spawn(async move { store_clone.close(&t()).await }));
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
        assert_eq!(store.seals(&t()).await.expect("seals").len(), 1);
    }

    /// Using a closed store is a lifecycle mistake, not evidence of tampering.
    ///
    /// It must not surface as [`AuditError::ChainIntegrity`]: a caller matching on that
    /// variant to raise a tamper alarm would fire on a shutdown-ordering bug.
    #[tokio::test]
    async fn should_refuse_to_append_after_close() {
        let store = make_store();
        store.close(&t()).await.expect("close");

        let err = store
            .append(&t(), vec![make_input("late")])
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
            .rotate(&t())
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
                    .append(&t(), inputs)
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
            .verify(&t())
            .await
            .expect("chain must verify after concurrent appends");

        let all_entries = store.entries(&t()).await.expect("entries");
        assert_eq!(all_entries.len(), TASKS * ENTRIES_PER_TASK);
    }

    // ── S1b: tenant isolation ───────────────────────────────────────────────

    /// Two tenants sharing one store must never observe each other's chain — the core
    /// claim of S1b §1.2 (chains partition by `(NodeId, TenantId)`). Same node, same
    /// process, deliberately: isolation must hold *within* one running store, not just
    /// across separately-provisioned ones.
    #[tokio::test]
    async fn two_tenants_audit_chains_are_independent() {
        let store = make_store();

        store
            .append(&t(), vec![make_input("a-1"), make_input("a-2")])
            .await
            .expect("tenant a append");
        store
            .append(&t2(), vec![make_input("b-1")])
            .await
            .expect("tenant b append");

        let a_entries = store.entries(&t()).await.expect("tenant a entries");
        let b_entries = store.entries(&t2()).await.expect("tenant b entries");
        assert_eq!(a_entries.len(), 2);
        assert_eq!(b_entries.len(), 1);
        assert_eq!(b_entries[0].id, "b-1");
        assert!(a_entries.iter().all(|e| e.id != "b-1"));

        // Independent tips, independent chains — appending to one must not move the
        // other's head.
        let a_tip_before = store.tip(&t()).await.unwrap();
        store
            .append(&t2(), vec![make_input("b-2")])
            .await
            .expect("tenant b second append");
        assert_eq!(store.tip(&t()).await.unwrap(), a_tip_before);

        // Rotating one tenant's chain must not seal or otherwise disturb the other's.
        store.rotate(&t()).await.expect("tenant a rotate");
        assert_eq!(store.seals(&t()).await.expect("tenant a seals").len(), 1);
        assert_eq!(store.seals(&t2()).await.expect("tenant b seals").len(), 0);
        assert_eq!(
            store.entries(&t2()).await.expect("tenant b entries").len(),
            2
        );

        // Each tenant's own chain still verifies independently.
        store.verify(&t()).await.expect("tenant a chain verifies");
        store.verify(&t2()).await.expect("tenant b chain verifies");
    }

    /// [`AuditStore::history`] must page one tenant's segments only — a tenant with a
    /// longer or differently-shaped run (more rotations, more entries) must never leak
    /// extents into another tenant's page.
    #[tokio::test]
    async fn audit_history_never_returns_another_tenants_entries() {
        let store = make_store();

        store
            .append(&t(), vec![make_input("a-1")])
            .await
            .expect("tenant a append");
        store.rotate(&t()).await.expect("tenant a rotate");
        store
            .append(&t(), vec![make_input("a-2")])
            .await
            .expect("tenant a append 2");

        store
            .append(&t2(), vec![make_input("b-1"), make_input("b-2")])
            .await
            .expect("tenant b append");

        let a_page = store
            .history(&t(), None, 100)
            .await
            .expect("tenant a history");
        let a_ids: Vec<&str> = a_page.entries.iter().map(|e| e.id.as_str()).collect();
        assert_eq!(a_ids, vec!["a-1", "a-2"]);

        let b_page = store
            .history(&t2(), None, 100)
            .await
            .expect("tenant b history");
        let b_ids: Vec<&str> = b_page.entries.iter().map(|e| e.id.as_str()).collect();
        assert_eq!(b_ids, vec!["b-1", "b-2"]);
    }
}
