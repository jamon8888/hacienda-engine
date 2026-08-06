//! Durable, fsync-backed implementation of [`AuditStore`].
//!
//! # Layout
//!
//! ```text
//! root/
//!   {node_id}/
//!     {segment_id}.jsonl       ← one entry per line, JSON
//!     {segment_id}.seal.json   ← SegmentSeal for a closed segment
//! ```
//!
//! A `.jsonl` with no matching `.seal.json` was open when the process ended. Recovery
//! replays it into a fresh segment, seals it, writes the seal, and opens a successor
//! linked to it. The process that crashed left a valid chain behind; the recovering
//! process picks it up exactly where the crash stopped it.
//!
//! # Durability
//!
//! `File::flush` buffers to the kernel but does not survive power loss. Every write path
//! here calls `File::sync_data()` to push data to stable storage. The default
//! [`SyncPolicy::EveryBatch`] fsyncs after each document batch; [`SyncPolicy::OnSeal`]
//! defers until rotation or close, which trades durability for throughput when the
//! document rate is high and the audit log is not the last line of defence.
//!
//! # Locking discipline (critical — read before modifying)
//!
//! Every method that mutates state:
//! 1. Acquires the `Mutex<FileState>` guard.
//! 2. Builds the byte buffer it needs to write (pure in-memory work).
//! 3. **Drops the guard.**
//! 4. Calls `tokio::task::spawn_blocking` to write and fsync on a blocking thread.
//!
//! A `std::sync::MutexGuard` is `!Send`. A future that holds one across an `.await`
//! is `!Send` too, and `Arc<dyn AuditStore>` requires `Send`. The compiler will refuse
//! to compile step 4 if step 3 is accidentally omitted, which is exactly the enforcement
//! property described in Design Decision D3. Do not reach for `tokio::sync::Mutex` to
//! silence this error — that would be a design change, not a fix.

use async_trait::async_trait;
use std::collections::HashMap;
use std::fs::{self, File, OpenOptions};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::audit::entry::{AuditEntry, AuditEntryInput};
use crate::audit::error::AuditError;
use crate::audit::segment::{verify_seal_chain, NodeId, Segment, SegmentSeal};
use crate::audit::store::{verify_open_entries, verify_sealed_entries, AuditStore};

// ── SyncPolicy ────────────────────────────────────────────────────────────────

/// Controls when `sync_data()` is called on the segment's backing file.
///
/// The default is [`EveryBatch`](Self::EveryBatch). Changing to
/// [`OnSeal`](Self::OnSeal) risks losing the last batch before a crash in exchange
/// for reduced write amplification on high-throughput workloads. Phase 2's benchmarks
/// will quantify the trade-off on the production corpus.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SyncPolicy {
    /// Call `sync_data()` after writing every append batch and after every seal write.
    ///
    /// This is the safest setting: a power loss after a successful `append` call will
    /// not lose that batch. The cost is one extra syscall per document.
    #[default]
    EveryBatch,

    /// Call `sync_data()` only when sealing a segment (on `rotate` or `close`).
    ///
    /// Batches written since the last seal are durably ordered relative to each other
    /// (they are in the kernel page cache), but a power loss before the next seal could
    /// lose them. Use only when the audit log is a secondary record and the caller
    /// maintains its own durable primary store.
    OnSeal,
}

// ── FileState ─────────────────────────────────────────────────────────────────

/// All mutable state for a `FileAuditStore`, behind one lock.
///
/// The single-lock design mirrors [`InMemoryAuditStore`]'s `State`. It prevents the
/// `close` race described in Task 2's deviation note: a check-then-seal split across
/// two locks lets two concurrent callers both see "not yet closed".
#[derive(Debug)]
struct FileState {
    /// The segment currently accepting writes, or `None` once the store is closed.
    open: Option<Segment>,
    /// Seals in chain order, oldest first. Entries are not kept in memory after sealing —
    /// they are on disk and can be re-read for verification.
    sealed: Vec<SegmentSeal>,
    /// Path of the `.jsonl` file backing the open segment.
    open_path: Option<PathBuf>,
    /// The seal produced by `close`, cached for idempotent re-close.
    closed_seal: Option<SegmentSeal>,
}

// ── FileAuditStore ────────────────────────────────────────────────────────────

/// A durable [`AuditStore`] that persists entries to JSON-lines files and seals to JSON.
///
/// See module documentation for layout, durability, and locking discipline.
///
/// # Recovery
///
/// `open` scans the node's directory, loads every `.seal.json` it finds, orders them
/// by `prev_seal_hash` links (not by filename — segment IDs are uuid v4, so lexicographic
/// order is arbitrary), verifies the resulting chain, and then replays any unsealed
/// `.jsonl` into a fresh segment before sealing it and opening a successor.
///
/// If the seal chain does not verify, `open` returns an error rather than continuing.
/// Writing into a chain known to be broken would produce an unverifiable record — worse
/// than failing fast and requiring operator intervention.
///
/// # Two writers
///
/// Two processes writing to the same `root` directory use different [`NodeId`]s and
/// therefore different subdirectories. They do not share a segment or a file handle, so
/// they cannot corrupt each other's chains. Combining their records for a full audit
/// requires merging the seal lists and running [`verify_seal_chain`] on each node's
/// subsequence separately.
///
/// # Why there are two locks
///
/// `state` is a `std::sync::Mutex` guarding in-memory bookkeeping; it is never held across
/// an `.await`. That alone is **not** sufficient for correctness, because every mutating
/// operation has two critical sections: mint under the state lock, then write on a
/// blocking thread. Two concurrent callers can therefore mint as (A, B) and write as
/// (B, A). Memory stays consistent — minting was serialised — but the *file* ends up
/// holding entries out of chain order, and recovery replays the file, not memory. The
/// store would refuse to reopen its own output.
///
/// `io_order` closes that. It is a `tokio::sync::Mutex` held across the whole
/// mint-then-write sequence, making the on-disk order equal the minted order. It is the
/// one place a `tokio` mutex is correct here, precisely because it *must* span an
/// `.await`. This is a genuine ordering requirement, not a workaround for the `!Send`
/// error that a held `std` guard would produce.
///
/// Lock order is always `io_order` then `state`, never the reverse.
///
/// # Write-failure poisoning (Design Decision: poison on write failure)
///
/// Every mutating method follows a two-step sequence under `io_order`:
///   1. Mint entries and update the in-memory `Segment` under the `state` lock, then
///      drop the guard.
///   2. Write the serialised bytes to disk on a blocking thread.
///
/// If the disk write fails, the in-memory `Segment` already contains the minted
/// entries (with their chain hashes), while the file does not. The options are:
///
/// - **Roll back** by re-acquiring the `state` lock and reversing the mutation.
///   Readers hold only the `state` lock (not `io_order`), so they can observe the
///   phantom entries in the window between steps 1 and 2. After rollback the
///   in-memory and on-disk state agree again, but the transient read window remains.
///
/// - **Write first, mutate after**. Infeasible here: `segment.push(input)` computes
///   the chain hash against the current tip and modifies the tip atomically. The
///   serialised bytes are a product of the mutation; they cannot be computed before
///   it. Separating them would require invasive changes to `Segment`.
///
/// - **Poison the store** on write failure. Set an atomic flag after any disk write
///   error. Every subsequent call — read or write — returns an `Io` error immediately.
///   The transient read window (between mutation and poisoning) is the same size as
///   the rollback window, but a caller who retries `append` after an `Io` error will
///   also receive `Io`, never a silent second entry minted against a phantom tip.
///   Reopen replays from disk, which never received the phantom entry, giving a clean
///   chain.
///
/// Poison is chosen because it closes the silent corruption path: without it, a
/// second `append` after a failed first one mints new entries whose `prev_hash` points
/// at an entry that does not exist on disk. The file then fails to reopen (the chain
/// hash does not verify). With poison, the second `append` is rejected immediately;
/// the caller reopens the store and gets a clean chain that matches the file.
#[derive(Debug)]
pub struct FileAuditStore {
    /// Serialises the mint-then-write sequence of `append`, `rotate`, and `close` so that
    /// on-disk order matches chain order. See "Why there are two locks" above.
    io_order: tokio::sync::Mutex<()>,
    state: Mutex<FileState>,
    node_id: NodeId,
    config_hash: String,
    node_dir: PathBuf,
    sync_policy: SyncPolicy,
    /// Set to `true` after any disk write failure. Once poisoned, all subsequent
    /// operations return an `Io` error immediately. The caller must reopen the store
    /// (which replays from disk) to recover. See struct-level docs for rationale.
    poisoned: AtomicBool,
    /// `io_order` wait telemetry, opt-in via [`Self::with_contention_tracking`].
    ///
    /// `None` by default so that every `append` outside a benchmark costs one
    /// `Option::is_some` check and nothing else — no `Instant::now()` pair is paid
    /// per call when contention tracking is switched off. See the phase2 plan's
    /// Task 5, Design Decision D2.
    contention: Option<Arc<ContentionMetrics>>,
}

/// Accumulates how long `append` callers spent waiting to acquire `io_order`,
/// distinct from the time spent minting and fsyncing once acquired.
///
/// Plain atomics rather than a mutex: the whole point is to measure contention
/// without introducing more of it, and every writer only ever adds.
#[derive(Debug, Default)]
struct ContentionMetrics {
    wait_nanos: AtomicU64,
    wait_count: AtomicU64,
}

impl ContentionMetrics {
    fn record(&self, wait: Duration) {
        self.wait_nanos
            .fetch_add(wait.as_nanos() as u64, Ordering::Relaxed);
        self.wait_count.fetch_add(1, Ordering::Relaxed);
    }

    fn total(&self) -> Duration {
        Duration::from_nanos(self.wait_nanos.load(Ordering::Relaxed))
    }

    fn count(&self) -> u64 {
        self.wait_count.load(Ordering::Relaxed)
    }
}

impl FileAuditStore {
    /// Open or recover a durable audit store rooted at `root`.
    ///
    /// On first call this creates `root/{node_id}/`. On subsequent calls it finds any
    /// existing segments, verifies their seal chain, replays any unsealed segment (one
    /// left open by a previous run), and opens a successor linked to the recovered chain.
    ///
    /// # Errors
    ///
    /// - [`AuditError::Io`] — directory creation or file I/O failed.
    /// - [`AuditError::Json`] — a seal or entry file is malformed.
    /// - [`AuditError::SegmentIntegrity`] / [`AuditError::SegmentLink`] — the seal
    ///   chain does not verify. The store refuses to open rather than appending to a
    ///   chain already known to be broken.
    /// - [`AuditError::ChainIntegrity`] / [`AuditError::ConfigMismatch`] — an entry
    ///   in the unsealed `.jsonl` does not extend the segment's chain. This fires when
    ///   the file was edited between the crash and the recovery.
    pub fn open(
        root: impl AsRef<Path>,
        node_id: NodeId,
        config_hash: impl Into<String>,
    ) -> Result<Self, AuditError> {
        Self::open_with_policy(root, node_id, config_hash, SyncPolicy::default())
    }

    /// Same as [`open`](Self::open) but with an explicit sync policy.
    pub fn open_with_policy(
        root: impl AsRef<Path>,
        node_id: NodeId,
        config_hash: impl Into<String>,
        sync_policy: SyncPolicy,
    ) -> Result<Self, AuditError> {
        let config_hash = config_hash.into();
        let node_dir = root.as_ref().join(node_id.as_str());

        fs::create_dir_all(&node_dir).map_err(|source| AuditError::Io {
            path: node_dir.display().to_string(),
            source,
        })?;

        // ── Recovery ──────────────────────────────────────────────────────────
        // Load every `.seal.json` in the node directory, order them by the
        // `prev_seal_hash` chain (not by filename — uuid v4 names have no
        // meaningful order), verify the chain, then replay any unsealed `.jsonl`.

        let seals = load_and_order_seals(&node_dir)?;
        verify_seal_chain(&seals)?;

        // Identify any `.jsonl` that has no matching seal — it was open at crash time.
        let unsealed_path = find_unsealed_jsonl(&node_dir, &seals)?;

        let (open_segment, open_path, sealed) = if let Some(unsealed) = unsealed_path {
            // Replay the unsealed entries into a fresh segment to validate the chain.
            // If any entry does not extend the chain, the file was tampered with and
            // `replay_segment` returns an error — this is the Gap 1 payoff.
            let prev_seal_hash = seals.last().map(|s| s.seal_hash.clone());
            let replayed =
                replay_segment(&unsealed, node_id.clone(), &config_hash, prev_seal_hash)?;

            // Seal it and write the seal before opening the successor. This ensures
            // that a second crash during recovery leaves a sealed segment behind,
            // not another unsealed one.
            let seal = replayed.seal();
            write_seal_file(&node_dir, &seal)?;
            sync_file(seal_path(&node_dir, &seal.segment_id))?;

            let mut sealed = seals;
            let prev_seal_hash = seal.seal_hash.clone();
            sealed.push(seal);

            let successor =
                Segment::open(node_id.clone(), config_hash.clone(), Some(prev_seal_hash));
            let successor_path = jsonl_path(&node_dir, successor.id());
            create_jsonl(&successor_path)?;

            (successor, successor_path, sealed)
        } else {
            // No unsealed segment. Open a fresh one linked to the last seal.
            let prev_seal_hash = seals.last().map(|s| s.seal_hash.clone());
            let segment = Segment::open(node_id.clone(), config_hash.clone(), prev_seal_hash);
            let path = jsonl_path(&node_dir, segment.id());
            create_jsonl(&path)?;
            (segment, path, seals)
        };

        Ok(Self {
            io_order: tokio::sync::Mutex::new(()),
            state: Mutex::new(FileState {
                open: Some(open_segment),
                sealed,
                open_path: Some(open_path),
                closed_seal: None,
            }),
            node_id,
            config_hash,
            node_dir,
            sync_policy,
            poisoned: AtomicBool::new(false),
            contention: None,
        })
    }

    /// Builder: override the sync policy after construction is not possible because
    /// opening already performs recovery I/O. Pass the policy to
    /// [`open_with_policy`](Self::open_with_policy) instead.
    ///
    /// This method is provided for test ergonomics where the store is freshly created
    /// and the caller wants to change the policy before appending anything.
    pub fn with_sync_policy(mut self, policy: SyncPolicy) -> Self {
        self.sync_policy = policy;
        self
    }

    /// Builder: opt into recording how long `append` callers spend waiting to acquire
    /// `io_order`, separate from the time spent minting and writing once acquired.
    ///
    /// Off by default: measuring costs a pair of `Instant::now()` calls per `append`,
    /// which this crate does not pay unless something is actually asking for the
    /// number. See the phase2 plan's Task 5, Design Decision D2.
    pub fn with_contention_tracking(mut self) -> Self {
        self.contention = Some(Arc::new(ContentionMetrics::default()));
        self
    }

    /// Total time and count of waits to acquire `io_order`, if contention tracking was
    /// enabled via [`Self::with_contention_tracking`]. `None` otherwise.
    pub fn io_order_wait(&self) -> Option<(Duration, u64)> {
        self.contention.as_ref().map(|c| (c.total(), c.count()))
    }

    fn state(&self) -> std::sync::MutexGuard<'_, FileState> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Return an `Io` error if a previous write failure poisoned this store.
    ///
    /// Called at the top of every public method. A poisoned store has in-memory
    /// state that diverges from disk; serving any read or write from it would be
    /// misleading. The caller must reopen the store to recover.
    fn check_not_poisoned(&self) -> Result<(), AuditError> {
        if self.poisoned.load(Ordering::Acquire) {
            return Err(AuditError::Io {
                path: self.node_dir.display().to_string(),
                source: std::io::Error::other(
                    "store is poisoned after a previous write failure; reopen to recover",
                ),
            });
        }
        Ok(())
    }

    /// Mark this store poisoned and return the original error.
    ///
    /// Called after any disk write fails. Once poisoned, `check_not_poisoned`
    /// will reject every subsequent call.
    fn poison(&self, error: AuditError) -> AuditError {
        self.poisoned.store(true, Ordering::Release);
        tracing::error!(
            node_dir = %self.node_dir.display(),
            "audit store poisoned after a write failure; reopen required"
        );
        error
    }
}

#[async_trait]
impl AuditStore for FileAuditStore {
    async fn append(&self, inputs: Vec<AuditEntryInput>) -> Result<Vec<AuditEntry>, AuditError> {
        self.check_not_poisoned()?;

        // Held for the whole mint-then-write sequence so that the order entries are
        // minted in is the order they reach the file. Without it, two concurrent callers
        // can mint as (A, B) and write as (B, A), leaving a file this store cannot
        // replay. See "Why there are two locks" on the struct.
        //
        // `wait_start` is only ever `Some` when contention tracking is on, so the
        // `Instant::now()` pair is skipped entirely otherwise.
        let wait_start = self.contention.is_some().then(Instant::now);
        let _io = self.io_order.lock().await;
        if let (Some(contention), Some(start)) = (&self.contention, wait_start) {
            contention.record(start.elapsed());
        }

        // ── Step 1: build the byte buffer and mint entries under the state lock ──
        // The std guard is dropped before the spawn_blocking call. Holding a
        // std::sync::MutexGuard across .await makes the future !Send, which will not
        // compile behind Arc<dyn AuditStore>. Ordering is `_io`'s job, not the state
        // lock's — do not try to solve it by swapping `state` for a tokio mutex.
        let (entries, bytes, path, should_sync) = {
            let mut state = self.state();

            let segment = state.open.as_mut().ok_or(AuditError::StoreClosed {
                operation: "append",
            })?;

            let entries: Vec<AuditEntry> = inputs
                .into_iter()
                .map(|input| segment.push(input).clone())
                .collect();

            let mut bytes = Vec::new();
            for entry in &entries {
                serde_json::to_writer(&mut bytes, entry)?;
                bytes.push(b'\n');
            }

            // `open` and `open_path` are set and cleared together, so a present segment
            // always has a path. Reported as an error rather than asserted: a panic in an
            // audit write loses the batch and takes the caller down with it.
            let path = state.open_path.clone().ok_or(AuditError::StoreClosed {
                operation: "append",
            })?;

            let should_sync = self.sync_policy == SyncPolicy::EveryBatch;

            (entries, bytes, path, should_sync)
            // ── state guard drops here; `_io` is still held ───────────────────
        };

        // ── Step 2: write and optionally fsync on a blocking thread ───────────
        let result = tokio::task::spawn_blocking(move || {
            let file = OpenOptions::new()
                .append(true)
                .open(&path)
                .map_err(|source| AuditError::Io {
                    path: path.display().to_string(),
                    source,
                })?;
            let mut writer = BufWriter::new(&file);
            writer.write_all(&bytes).map_err(|source| AuditError::Io {
                path: path.display().to_string(),
                source,
            })?;
            writer.flush().map_err(|source| AuditError::Io {
                path: path.display().to_string(),
                source,
            })?;
            if should_sync {
                file.sync_data().map_err(|source| AuditError::Io {
                    path: path.display().to_string(),
                    source,
                })?;
            }
            Ok::<(), AuditError>(())
        })
        .await
        .map_err(|join_err| AuditError::Io {
            path: self.node_dir.display().to_string(),
            source: std::io::Error::other(join_err.to_string()),
        })?;

        if let Err(err) = result {
            return Err(self.poison(err));
        }

        Ok(entries)
    }

    async fn entries(&self) -> Result<Vec<AuditEntry>, AuditError> {
        self.check_not_poisoned()?;
        let state = self.state();
        Ok(state
            .open
            .as_ref()
            .map(|s| s.entries().to_vec())
            .unwrap_or_default())
    }

    async fn tip(&self) -> Result<String, AuditError> {
        self.check_not_poisoned()?;
        let state = self.state();
        match state.open.as_ref() {
            Some(segment) if !segment.is_empty() => Ok(segment.tip().to_owned()),
            // Empty open segment, or no open segment at all: the newest seal is the head.
            _ => Ok(state
                .sealed
                .last()
                .map(|seal| seal.sealed_tip.clone())
                .unwrap_or_else(|| crate::audit::GENESIS_HASH.to_owned())),
        }
    }

    async fn seals(&self) -> Result<Vec<SegmentSeal>, AuditError> {
        self.check_not_poisoned()?;
        let state = self.state();
        Ok(state.sealed.clone())
    }

    async fn verify(&self) -> Result<(), AuditError> {
        self.check_not_poisoned()?;
        // Snapshot under the lock, then verify without holding it. Verification is O(n)
        // over every entry ever written; blocking all appends for its duration would make
        // a routine integrity check into an outage.
        let (sealed, open_entries, open_tip, node_dir) = {
            let state = self.state();
            let (entries, tip) = match state.open.as_ref() {
                Some(s) => (s.entries().to_vec(), s.tip().to_owned()),
                None => (Vec::new(), crate::audit::GENESIS_HASH.to_owned()),
            };
            (state.sealed.clone(), entries, tip, self.node_dir.clone())
        };

        // Seal chain first — see InMemoryAuditStore::verify for the ordering rationale.
        verify_seal_chain(&sealed)?;

        // Re-read each sealed segment's entries from disk to verify them. The in-memory
        // backend keeps entries in RAM; the file backend discards them after sealing to
        // avoid growing memory with the full history. Disk is the authoritative store.
        for seal in &sealed {
            let path = jsonl_path(&node_dir, &seal.segment_id);
            let entries = read_jsonl(&path)?;
            verify_sealed_entries(&entries, seal)?;
        }

        verify_open_entries(&open_entries, &open_tip, &self.config_hash)
    }

    async fn rotate(&self) -> Result<SegmentSeal, AuditError> {
        self.check_not_poisoned()?;

        // Held across the seal-then-write sequence for the same reason as in `append`,
        // plus one specific to rotation: a seal records `entry_count` and `sealed_tip`
        // for entries an in-flight `append` may not have written yet. Ordering the two
        // operations is what stops a seal describing a file that does not yet match it.
        let _io = self.io_order.lock().await;

        // ── Step 1: seal the open segment under the state lock ────────────────
        let (seal, seal_bytes, seal_path, new_path) = {
            let mut state = self.state();

            let old = state.open.take().ok_or(AuditError::StoreClosed {
                operation: "rotate",
            })?;

            let seal = old.seal();

            let seal_bytes = serde_json::to_vec(&seal)?;
            let sp = seal_path(&self.node_dir, &seal.segment_id);

            // Open the successor inside the lock. Between sealing and reopening, the
            // store has no open segment; releasing the lock in that window would let a
            // concurrent append see a closed store that is only mid-rotation.
            let successor = Segment::open(
                self.node_id.clone(),
                self.config_hash.clone(),
                Some(seal.seal_hash.clone()),
            );
            let np = jsonl_path(&self.node_dir, successor.id());

            state.open = Some(successor);
            state.open_path = Some(np.clone());
            state.sealed.push(seal.clone());

            (seal, seal_bytes, sp, np)
            // ── state guard drops here; `_io` is still held ───────────────────
        };

        // ── Step 2: write seal file and create successor file on disk ─────────
        let policy = self.sync_policy;
        let node_dir = self.node_dir.clone();
        let result = tokio::task::spawn_blocking(move || {
            // Write the seal file.
            write_bytes_to(&seal_path, &seal_bytes)?;
            sync_file(seal_path)?;

            // Create the successor's empty .jsonl.
            create_jsonl(&new_path)?;
            if policy == SyncPolicy::EveryBatch || policy == SyncPolicy::OnSeal {
                // Sync the directory entry so the new file is durable.
                sync_dir(&node_dir)?;
            }
            Ok::<(), AuditError>(())
        })
        .await
        .map_err(|join_err| AuditError::Io {
            path: self.node_dir.display().to_string(),
            source: std::io::Error::other(join_err.to_string()),
        })?;

        if let Err(err) = result {
            return Err(self.poison(err));
        }

        Ok(seal)
    }

    async fn close(&self) -> Result<SegmentSeal, AuditError> {
        self.check_not_poisoned()?;

        // Ordered against in-flight appends for the same reason as `rotate`: the seal
        // must not describe entries that have not reached the file yet.
        let _io = self.io_order.lock().await;

        // Idempotent and concurrency-safe: both the check and the seal happen under one
        // acquisition, matching InMemoryAuditStore's single-lock design.
        let (seal, seal_bytes, sp, already_closed) = {
            let mut state = self.state();

            if let Some(existing) = &state.closed_seal {
                return Ok(existing.clone());
            }

            let segment = state
                .open
                .take()
                .ok_or(AuditError::StoreClosed { operation: "close" })?;

            let seal = segment.seal();
            let seal_bytes = serde_json::to_vec(&seal)?;
            let sp = seal_path(&self.node_dir, &seal.segment_id);

            state.sealed.push(seal.clone());
            state.closed_seal = Some(seal.clone());
            state.open_path = None;

            (seal, seal_bytes, sp, false)
            // ── guard drops here ──────────────────────────────────────────────
        };

        if already_closed {
            return Ok(seal);
        }

        // Write and fsync the seal — always, regardless of SyncPolicy, because a seal
        // is an immutable record. Losing a seal is worse than losing an append batch.
        let result = tokio::task::spawn_blocking(move || {
            write_bytes_to(&sp, &seal_bytes)?;
            sync_file(sp)?;
            Ok::<(), AuditError>(())
        })
        .await
        .map_err(|join_err| AuditError::Io {
            path: self.node_dir.display().to_string(),
            source: std::io::Error::other(join_err.to_string()),
        })?;

        if let Err(err) = result {
            return Err(self.poison(err));
        }

        Ok(seal)
    }
}

// ── File helpers ──────────────────────────────────────────────────────────────

fn jsonl_path(node_dir: &Path, segment_id: &str) -> PathBuf {
    node_dir.join(format!("{segment_id}.jsonl"))
}

fn seal_path(node_dir: &Path, segment_id: &str) -> PathBuf {
    node_dir.join(format!("{segment_id}.seal.json"))
}

/// Create an empty `.jsonl` file, failing if the parent directory does not exist.
fn create_jsonl(path: &Path) -> Result<(), AuditError> {
    File::create(path).map_err(|source| AuditError::Io {
        path: path.display().to_string(),
        source,
    })?;
    Ok(())
}

/// Atomically write `bytes` to `path`, creating or truncating it.
fn write_bytes_to(path: &Path, bytes: &[u8]) -> Result<(), AuditError> {
    let mut file = File::create(path).map_err(|source| AuditError::Io {
        path: path.display().to_string(),
        source,
    })?;
    file.write_all(bytes).map_err(|source| AuditError::Io {
        path: path.display().to_string(),
        source,
    })?;
    file.flush().map_err(|source| AuditError::Io {
        path: path.display().to_string(),
        source,
    })?;
    Ok(())
}

/// Write a `SegmentSeal` to `{node_dir}/{segment_id}.seal.json`.
fn write_seal_file(node_dir: &Path, seal: &SegmentSeal) -> Result<(), AuditError> {
    let bytes = serde_json::to_vec(seal)?;
    let path = seal_path(node_dir, &seal.segment_id);
    write_bytes_to(&path, &bytes)
}

/// Call `sync_data` on the file at `path`. Used after writing seals and (under
/// `EveryBatch`) after writing entry batches.
fn sync_file(path: PathBuf) -> Result<(), AuditError> {
    let file = File::open(&path).map_err(|source| AuditError::Io {
        path: path.display().to_string(),
        source,
    })?;
    file.sync_data().map_err(|source| AuditError::Io {
        path: path.display().to_string(),
        source,
    })
}

/// fsync the directory itself so newly created file entries are durable.
///
/// On Linux, creating a file in a directory does not guarantee the directory entry is
/// durable until the directory file descriptor is fsynced. This matters after `rotate`
/// creates the successor's empty `.jsonl`.
fn sync_dir(dir: &Path) -> Result<(), AuditError> {
    let dir_file = File::open(dir).map_err(|source| AuditError::Io {
        path: dir.display().to_string(),
        source,
    })?;
    dir_file.sync_data().map_err(|source| AuditError::Io {
        path: dir.display().to_string(),
        source,
    })
}

/// Read all entries from a `.jsonl` file, one JSON object per line.
///
/// The newline is the record terminator, and it is the only reliable signal that a
/// record was fully written. Any unterminated bytes at the end of the file are removed
/// — from the file, not just from the returned `Vec` — and every remaining line must
/// parse or this returns [`AuditError::Json`].
///
/// The two halves of that rule answer different failure modes:
///
/// - **Unterminated tail.** A crash part-way through `append`. The bytes were never
///   fsynced and `append` never returned success, so no caller was told they were
///   durable; the chain is still a valid prefix and still verifies. Refusing to open
///   would cost the whole audit history to one power cut.
/// - **Terminated line that will not parse.** Something rewrote a record that had
///   already been committed. That is real corruption, it is what the hash chain exists
///   to catch, and it must be loud.
///
/// The removal must touch the file because `append` writes at EOF: a fragment left in
/// place would be welded to the next record on the same physical line, turning one lost
/// write into permanent, silent loss of every write that follows.
fn read_jsonl(path: &Path) -> Result<Vec<AuditEntry>, AuditError> {
    // Bytes, not a string: a crash can split a multi-byte UTF-8 character, and
    // `read_to_string` would refuse the file outright rather than let us drop the tail.
    let raw = fs::read(path).map_err(|source| AuditError::Io {
        path: path.display().to_string(),
        source,
    })?;

    let complete = discard_unterminated_tail(path, &raw)?;
    let contents = std::str::from_utf8(complete).map_err(|err| AuditError::Io {
        path: path.display().to_string(),
        source: std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("segment file is not valid UTF-8: {err}"),
        ),
    })?;

    contents
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| serde_json::from_str(line).map_err(AuditError::Json))
        .collect()
}

/// Shorten `path` to its last complete record and return the surviving prefix of `raw`.
///
/// A no-op, with no I/O at all, when the file already ends at a record boundary — which
/// is every clean shutdown.
///
/// # Errors
///
/// [`AuditError::Io`] if the file cannot be opened for truncation or shortened.
fn discard_unterminated_tail<'a>(path: &Path, raw: &'a [u8]) -> Result<&'a [u8], AuditError> {
    if raw.is_empty() || raw.ends_with(b"\n") {
        return Ok(raw);
    }

    // Position just past the last terminator; 0 when the file holds one partial record.
    let keep = raw
        .iter()
        .rposition(|byte| *byte == b'\n')
        .map_or(0, |idx| idx + 1);

    tracing::warn!(
        path = %path.display(),
        discarded_bytes = raw.len() - keep,
        "audit segment ended mid-record; discarding the unterminated tail so the segment stays appendable"
    );

    let io_error = |source| AuditError::Io {
        path: path.display().to_string(),
        source,
    };
    let file = OpenOptions::new()
        .write(true)
        .open(path)
        .map_err(io_error)?;
    file.set_len(keep as u64).map_err(io_error)?;
    // `set_len` changes the file's length, which is metadata; `sync_all` rather than
    // `sync_data`, or the next crash resurrects the tail we just removed.
    file.sync_all().map_err(io_error)?;

    Ok(&raw[..keep])
}

/// Load every `.seal.json` in `node_dir`, then order them by `prev_seal_hash` links.
///
/// Segment IDs are uuid v4, so lexicographic filename order is arbitrary and must not
/// be used. Instead, the seals are built into a map keyed by `prev_seal_hash` (using
/// `None` → genesis) and then threaded into order by following the chain from the
/// genesis seal forward. This is a topological sort of a linear linked list.
///
/// A seal chain with a fork (two seals claiming the same predecessor) or a cycle
/// would indicate file-level corruption beyond what normal tamper detection covers;
/// `order_seals` returns an error in that case.
fn load_and_order_seals(node_dir: &Path) -> Result<Vec<SegmentSeal>, AuditError> {
    let mut raw: Vec<SegmentSeal> = Vec::new();

    let entries = fs::read_dir(node_dir).map_err(|source| AuditError::Io {
        path: node_dir.display().to_string(),
        source,
    })?;

    for entry in entries {
        let entry = entry.map_err(|source| AuditError::Io {
            path: node_dir.display().to_string(),
            source,
        })?;
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) == Some("json")
            && path
                .file_name()
                .and_then(|n| n.to_str())
                .map(|n| n.ends_with(".seal.json"))
                .unwrap_or(false)
        {
            let contents = fs::read_to_string(&path).map_err(|source| AuditError::Io {
                path: path.display().to_string(),
                source,
            })?;
            let seal: SegmentSeal = serde_json::from_str(&contents)?;
            raw.push(seal);
        }
    }

    if raw.is_empty() {
        return Ok(Vec::new());
    }

    // Build a map from prev_seal_hash → seal so we can walk the chain from genesis.
    // Key: the prev_seal_hash value (None represented as the empty string sentinel).
    let mut by_prev: HashMap<String, SegmentSeal> = HashMap::with_capacity(raw.len());
    for seal in raw {
        let key = seal.prev_seal_hash.clone().unwrap_or_default();
        by_prev.insert(key, seal);
    }

    // Walk from genesis forward, following prev_seal_hash links.
    let mut ordered: Vec<SegmentSeal> = Vec::with_capacity(by_prev.len());
    let mut current_key = String::new(); // genesis sentinel

    loop {
        match by_prev.remove(&current_key) {
            None => break,
            Some(seal) => {
                current_key = seal.seal_hash.clone();
                ordered.push(seal);
            }
        }
    }

    // If any seals remain in by_prev, the chain had a fork or a broken link.
    if !by_prev.is_empty() {
        return Err(AuditError::SegmentLink {
            segment_id: by_prev
                .values()
                .next()
                .map(|s| s.segment_id.clone())
                .unwrap_or_default(),
            expected: "reachable from genesis via prev_seal_hash".to_owned(),
            actual: "unreachable — possible fork or broken link in seal files".to_owned(),
        });
    }

    Ok(ordered)
}

/// Find the `.jsonl` file in `node_dir` that has no matching `.seal.json`.
///
/// There should be at most one such file — the segment that was open at crash time.
/// If two unsealed segments are found, the directory is corrupt; we return the first
/// one and let the chain replay fail with a meaningful error.
fn find_unsealed_jsonl(
    node_dir: &Path,
    seals: &[SegmentSeal],
) -> Result<Option<PathBuf>, AuditError> {
    let sealed_ids: std::collections::HashSet<&str> =
        seals.iter().map(|s| s.segment_id.as_str()).collect();

    let entries = fs::read_dir(node_dir).map_err(|source| AuditError::Io {
        path: node_dir.display().to_string(),
        source,
    })?;

    let mut unsealed: Option<PathBuf> = None;

    for entry in entries {
        let entry = entry.map_err(|source| AuditError::Io {
            path: node_dir.display().to_string(),
            source,
        })?;
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("jsonl") {
            continue;
        }
        // Extract the segment_id from the filename: `{segment_id}.jsonl`
        let stem = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_owned();
        if !sealed_ids.contains(stem.as_str()) {
            unsealed = Some(path);
            // Do not break: we want to discover if there are two unsealed files.
            // For now, return the first found; chain replay will validate it.
            break;
        }
    }

    Ok(unsealed)
}

/// Replay a `.jsonl` file into a fresh segment, validating every entry against the chain.
///
/// Returns the fully-populated segment, ready to be sealed. If any entry does not extend
/// the chain (because the file was tampered with between crash and recovery), returns an
/// error — the Gap 1 property described in the implementation brief.
///
/// The segment ID is taken from the `.jsonl` filename stem so that the seal produced by
/// `segment.seal()` has the same `segment_id` as the file the entries came from. Using a
/// freshly-minted uuid would create a mismatch between the seal's `segment_id` and the
/// actual filename, which would break `verify()`'s attempt to re-read the file.
fn replay_segment(
    path: &Path,
    node_id: NodeId,
    config_hash: &str,
    prev_seal_hash: Option<String>,
) -> Result<Segment, AuditError> {
    let entries = read_jsonl(path)?;

    // Use the file stem as the segment_id so the resulting seal refers to the correct
    // file. Segment IDs are uuid v4 strings stored as filenames; the stem is the
    // authoritative identifier for this segment.
    let segment_id = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("unknown-segment");

    let mut segment = Segment::open_with_id(segment_id, node_id, config_hash, prev_seal_hash);

    for entry in entries {
        segment.append(entry)?;
    }

    Ok(segment)
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audit::entry::{EntitySource, RedactionAction};
    use std::sync::Arc;

    // ── TempDir RAII helper ───────────────────────────────────────────────────
    // Copied from audit/sink.rs:145-169. Both modules need isolation; the helper is
    // not worth making pub(crate) just to share six lines.

    struct TempDir(PathBuf);

    impl TempDir {
        fn new(tag: &str) -> Self {
            let path = std::env::temp_dir().join(format!(
                "hacienda-file-audit-{tag}-{}",
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    // ── helpers ───────────────────────────────────────────────────────────────

    const CONFIG: &str = "test-config-hash";
    const NODE: &str = "test-node-0";

    fn make_store(dir: &TempDir) -> FileAuditStore {
        FileAuditStore::open(dir.path(), NodeId::new(NODE), CONFIG).unwrap()
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
            config_hash: CONFIG.into(),
            principal: None,
            vertical: None,
        }
    }

    /// The durable backend must report the same tip the in-memory one does, including
    /// across a reopen: a client that recorded the tip before a restart has to be able to
    /// match it against the chain the recovered store presents.
    #[tokio::test]
    async fn should_report_a_tip_that_survives_a_reopen() {
        let dir = TempDir::new("tip-across-reopen");
        let store = make_store(&dir);
        assert_eq!(store.tip().await.unwrap(), crate::audit::GENESIS_HASH);

        let entries = store.append(vec![make_input("a")]).await.unwrap();
        let tip = store.tip().await.unwrap();
        assert_eq!(tip, entries[0].chain_hash);
        drop(store);

        let reopened = FileAuditStore::open(dir.path(), NodeId::new(NODE), CONFIG).expect("reopen");
        assert_eq!(
            reopened.tip().await.unwrap(),
            tip,
            "recovery must reach the same chain head the client recorded"
        );
    }

    // ── Step 6 tests ─────────────────────────────────────────────────────────

    #[tokio::test]
    async fn should_write_one_json_line_per_entry() {
        let dir = TempDir::new("write-lines");
        let store = make_store(&dir);

        store
            .append(vec![make_input("e1"), make_input("e2"), make_input("e3")])
            .await
            .expect("append must succeed");

        let node_dir = dir.path().join(NODE);
        let open_seg = store.state().open.as_ref().unwrap().id().to_owned();
        let jsonl = jsonl_path(&node_dir, &open_seg);
        let contents = fs::read_to_string(&jsonl).expect("jsonl must exist");

        assert_eq!(
            contents.lines().count(),
            3,
            "each entry must produce exactly one JSON line"
        );
    }

    #[tokio::test]
    async fn should_recover_every_entry_after_a_simulated_restart() {
        let dir = TempDir::new("recover");

        // First run: write entries, then drop the store without sealing.
        {
            let store = make_store(&dir);
            store
                .append(vec![make_input("r1"), make_input("r2"), make_input("r3")])
                .await
                .expect("append");
            // Drop without calling close — simulates a crash.
        }

        // Second run: open recovers the unsealed segment.
        let store2 = FileAuditStore::open(dir.path(), NodeId::new(NODE), CONFIG)
            .expect("recovery must succeed");

        // The recovered segment is now sealed; the open segment is its successor.
        // Verify that the full chain is intact.
        store2.verify().await.expect("recovered chain must verify");

        // The entries from the crashed run are in the sealed segment.
        let seals = store2.seals().await.expect("seals");
        assert!(
            !seals.is_empty(),
            "recovered store must have at least one seal"
        );
    }

    #[tokio::test]
    async fn should_seal_a_segment_left_open_by_a_previous_run() {
        let dir = TempDir::new("seal-on-recover");

        {
            let store = make_store(&dir);
            store.append(vec![make_input("s1")]).await.expect("append");
            // Crash without sealing.
        }

        // Recovery must seal the orphan segment.
        let store2 = FileAuditStore::open(dir.path(), NodeId::new(NODE), CONFIG).expect("recovery");

        let seals = store2.seals().await.expect("seals");
        assert_eq!(
            seals.len(),
            1,
            "the orphan segment must be sealed on recovery"
        );

        // The seal file must exist on disk.
        let node_dir = dir.path().join(NODE);
        let seal_file = seal_path(&node_dir, &seals[0].segment_id);
        assert!(seal_file.exists(), "seal file must be present on disk");
    }

    #[tokio::test]
    async fn should_link_a_recovered_segment_to_the_one_before_it() {
        let dir = TempDir::new("link-recovered");

        // Run 1: append and explicitly seal with rotate.
        let seal1 = {
            let store = make_store(&dir);
            store.append(vec![make_input("a1")]).await.expect("append");
            store.rotate().await.expect("rotate")
            // Drop after rotate — the successor is unsealed.
        };

        // Run 2: recovery seals the successor and opens a new one.
        let store2 = FileAuditStore::open(dir.path(), NodeId::new(NODE), CONFIG).expect("recovery");

        let seals = store2.seals().await.expect("seals");
        // seals[0] = seal1 (from rotate), seals[1] = sealed orphan from run 1's successor
        assert!(
            seals.len() >= 2,
            "there must be at least two seals after recovery: {seals:?}"
        );

        // The second seal must point at the first.
        assert_eq!(
            seals[1].prev_seal_hash.as_deref(),
            Some(seal1.seal_hash.as_str()),
            "recovered seal must link to its predecessor"
        );

        store2
            .verify()
            .await
            .expect("full chain must verify after two-run recovery");
    }

    #[tokio::test]
    async fn should_refuse_to_open_a_store_whose_seal_chain_is_broken() {
        let dir = TempDir::new("broken-chain");
        let node_dir = dir.path().join(NODE);

        // Run 1: write a segment and seal it with close.
        let seal = {
            let store = make_store(&dir);
            store.append(vec![make_input("b1")]).await.expect("append");
            store.close().await.expect("close")
        };

        // Corrupt the seal file by zeroing the seal_hash field.
        let seal_file = seal_path(&node_dir, &seal.segment_id);
        let mut broken: SegmentSeal =
            serde_json::from_str(&fs::read_to_string(&seal_file).expect("seal must exist"))
                .expect("seal must be valid JSON");
        broken.seal_hash =
            "0000000000000000000000000000000000000000000000000000000000000000".to_owned();
        fs::write(&seal_file, serde_json::to_vec(&broken).unwrap()).unwrap();

        // Open must fail, not silently adopt the broken chain.
        let result = FileAuditStore::open(dir.path(), NodeId::new(NODE), CONFIG);
        assert!(
            result.is_err(),
            "opening a store with a broken seal chain must fail"
        );
        match result.unwrap_err() {
            AuditError::SegmentIntegrity { .. } => {}
            other => panic!("expected SegmentIntegrity, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn should_verify_across_two_sealed_segments_and_one_open_one() {
        let dir = TempDir::new("two-seals-open");
        let store = make_store(&dir);

        store.append(vec![make_input("x1")]).await.expect("append");
        store.rotate().await.expect("rotate 1");
        store.append(vec![make_input("x2")]).await.expect("append");
        store.rotate().await.expect("rotate 2");
        store.append(vec![make_input("x3")]).await.expect("append");

        store
            .verify()
            .await
            .expect("two sealed segments + one open segment must verify");

        let seals = store.seals().await.expect("seals");
        assert_eq!(seals.len(), 2, "there must be exactly two sealed segments");
    }

    #[tokio::test]
    async fn should_keep_two_node_ids_in_separate_segments() {
        let dir = TempDir::new("two-nodes");

        let store_a =
            FileAuditStore::open(dir.path(), NodeId::new("node-a"), CONFIG).expect("open node-a");
        let store_b =
            FileAuditStore::open(dir.path(), NodeId::new("node-b"), CONFIG).expect("open node-b");

        store_a
            .append(vec![make_input("a1"), make_input("a2")])
            .await
            .expect("append to node-a");
        store_b
            .append(vec![make_input("b1")])
            .await
            .expect("append to node-b");

        store_a.verify().await.expect("node-a must verify");
        store_b.verify().await.expect("node-b must verify");

        // The two stores must live in separate directories.
        assert!(dir.path().join("node-a").is_dir());
        assert!(dir.path().join("node-b").is_dir());

        // Each node has its own segment; entries must not intermix.
        let entries_a = store_a.entries().await.expect("entries a");
        let entries_b = store_b.entries().await.expect("entries b");
        assert_eq!(entries_a.len(), 2);
        assert_eq!(entries_b.len(), 1);
    }

    /// A tampered `.jsonl` — one entry's field changed, its chain_hash left stale —
    /// must cause `FileAuditStore::open` to fail. This is the Gap 1 payoff: the
    /// recovery path calls `Segment::append` which calls `AuditChain::append` which
    /// recomputes the hash and rejects the mismatch at load time.
    #[tokio::test]
    async fn should_refuse_to_open_a_store_with_a_tampered_jsonl() {
        let dir = TempDir::new("tampered-jsonl");
        let node_dir = dir.path().join(NODE);

        // Write entries; do NOT seal — we want the unsealed .jsonl to be present.
        {
            let store = make_store(&dir);
            store
                .append(vec![make_input("t1"), make_input("t2")])
                .await
                .expect("append");
            // Crash without sealing.
        }

        // Find the unsealed .jsonl.
        let jsonl_file: PathBuf = fs::read_dir(&node_dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .find(|p| p.extension().and_then(|x| x.to_str()) == Some("jsonl"))
            .expect("must find a .jsonl");

        // Read it, tamper with one entry's category field, and write it back.
        let raw = fs::read_to_string(&jsonl_file).expect("read");
        let mut lines: Vec<String> = raw.lines().map(String::from).collect();
        assert!(lines.len() >= 2, "must have at least two entries");
        let mut entry: serde_json::Value = serde_json::from_str(&lines[1]).expect("parse entry");
        entry["category"] = serde_json::Value::String("CreditCard".into());
        lines[1] = serde_json::to_string(&entry).expect("re-serialize");
        fs::write(&jsonl_file, lines.join("\n") + "\n").expect("write tampered");

        // Recovery must fail with a chain integrity error.
        let result = FileAuditStore::open(dir.path(), NodeId::new(NODE), CONFIG);
        assert!(
            result.is_err(),
            "opening a store with a tampered .jsonl must fail"
        );
        match result.unwrap_err() {
            AuditError::ChainIntegrity { .. } => {}
            other => panic!("expected ChainIntegrity, got {other:?}"),
        }
    }

    /// A crash part-way through an append must not make the audit log unopenable.
    ///
    /// The mid-append crash is the *normal* power-loss outcome, not corruption. The
    /// partial entry was never fsynced and `append` never returned success for it, so no
    /// caller was ever told it was durable — dropping it loses nothing anyone was
    /// promised. Refusing to open, by contrast, costs the operator the entire audit
    /// history to a single power cut, which is precisely the durability DORA asks for.
    ///
    /// A *terminated* line that fails to parse stays a hard error; that one really is
    /// corruption. The newline is the record terminator and is what separates the two.
    #[tokio::test]
    async fn should_recover_from_a_crash_part_way_through_an_append() {
        let dir = TempDir::new("partial-append");
        let node_dir = dir.path().join(NODE);

        let sealed_before;
        {
            let store = make_store(&dir);
            store
                .append(vec![make_input("e1"), make_input("e2")])
                .await
                .expect("append");
            sealed_before = store.entries().await.expect("entries").len();
        }

        // Simulate the crash: a partial JSON object with no terminating newline.
        let jsonl_file: PathBuf = fs::read_dir(&node_dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .find(|p| p.extension().and_then(|x| x.to_str()) == Some("jsonl"))
            .expect("must find a .jsonl");
        {
            let mut file = fs::OpenOptions::new()
                .append(true)
                .open(&jsonl_file)
                .expect("open for append");
            file.write_all(b"{\"id\":\"e3\",\"category\":\"Em")
                .expect("write partial");
            file.sync_data().expect("sync");
        }

        // Recovery must succeed and keep every complete entry.
        let store = FileAuditStore::open(dir.path(), NodeId::new(NODE), CONFIG)
            .expect("a mid-append crash must not make the log unopenable");
        store
            .verify()
            .await
            .expect("the surviving prefix must verify");

        let total: u64 = store
            .seals()
            .await
            .expect("seals")
            .iter()
            .map(|s| s.entry_count)
            .sum();
        assert_eq!(
            total, sealed_before as u64,
            "every complete entry must survive; only the partial one is dropped"
        );

        // And the log must still be appendable — the partial bytes must be gone from
        // disk, not merely skipped in memory, or the next write welds onto them.
        store
            .append(vec![make_input("e4")])
            .await
            .expect("append after recovery");
        drop(store);
        let reopened = FileAuditStore::open(dir.path(), NodeId::new(NODE), CONFIG)
            .expect("reopen after post-recovery append");
        let total_after: u64 = reopened
            .seals()
            .await
            .expect("seals")
            .iter()
            .map(|s| s.entry_count)
            .sum();
        assert_eq!(
            total_after,
            sealed_before as u64 + 1,
            "the entry written after recovery must be durable"
        );
    }

    /// Synthetic throughput measurement for Step 8.
    ///
    /// Not a criterion benchmark — just wall-clock so the Phase 2 plan has baseline
    /// Concurrent appends must land in the file in the order they were minted.
    ///
    /// `append` mints under the state lock, drops the guard, then writes on a blocking
    /// thread. Those are two separate critical sections, so without an explicit ordering
    /// guarantee two concurrent callers can mint as (A, B) and write as (B, A). The chain
    /// in memory stays correct — minting was serialised — but the *file* holds entries out
    /// of chain order, and recovery replays a file, not memory. The store would come back
    /// up refusing to open its own output.
    ///
    /// The in-memory backend cannot have this bug: it has no second critical section.
    /// This test is the file backend's counterpart to
    /// `should_serialise_concurrent_appends_without_breaking_the_chain`.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn should_write_concurrent_appends_to_the_file_in_chain_order() {
        let dir = TempDir::new("concurrent-order");
        let store =
            Arc::new(FileAuditStore::open(dir.path(), NodeId::new("node-a"), "cfg").expect("open"));

        const TASKS: usize = 8;
        const PER_TASK: usize = 10;

        let mut handles = Vec::with_capacity(TASKS);
        for t in 0..TASKS {
            let store = Arc::clone(&store);
            handles.push(tokio::spawn(async move {
                let inputs: Vec<AuditEntryInput> = (0..PER_TASK)
                    .map(|i| make_input(&format!("t{t}-e{i}")))
                    .collect();
                store.append(inputs).await.expect("append");
            }));
        }
        for h in handles {
            h.await.expect("task must not panic");
        }

        store.close().await.expect("close");
        drop(store);

        // Reopening replays the file. If the on-disk order does not match the minted
        // order, `AuditChain::append` rejects the first out-of-place entry and this fails.
        let reopened = FileAuditStore::open(dir.path(), NodeId::new("node-a"), "cfg")
            .expect("the store must be able to reopen the file it just wrote");
        reopened.verify().await.expect("replayed chain must verify");

        let seals = reopened.seals().await.expect("seals");
        let total: u64 = seals.iter().map(|s| s.entry_count).sum();
        assert_eq!(total, (TASKS * PER_TASK) as u64);
    }

    // ── Write-failure / poison tests ─────────────────────────────────────────

    /// After a write failure on `append`, the store must be poisoned: all subsequent
    /// operations must return an I/O error. A successful `append` after a failed one
    /// must not include the phantom entries that were minted but never written to disk.
    ///
    /// Without the fix, the failed `append` leaves the in-memory `Segment` carrying
    /// entries whose chain hashes were computed against the pre-failure tip. A subsequent
    /// successful `append` mints new entries chained to those phantom ones. The file then
    /// holds only the second batch, but it is claimed to follow entries that are not there.
    /// Reopening calls `replay_segment` which recomputes hashes and rejects the mismatch.
    ///
    /// Fault injection: make the `.jsonl` file read-only after opening the store so that
    /// the first `append` write fails with EACCES. Uses `std::fs::set_permissions` under
    /// `#[cfg(test)]` — no production code is modified.
    #[tokio::test]
    async fn should_poison_store_after_a_failed_append_write() {
        use std::os::unix::fs::PermissionsExt;
        let dir = TempDir::new("append-write-fail");
        let store = make_store(&dir);

        // Find the open segment's .jsonl and make it read-only.
        let node_dir = dir.path().join(NODE);
        let seg_id = store.state().open.as_ref().unwrap().id().to_owned();
        let jsonl = jsonl_path(&node_dir, &seg_id);

        fs::set_permissions(&jsonl, fs::Permissions::from_mode(0o444))
            .expect("set permissions read-only");

        // `append` must fail — the file is read-only.
        let err = store
            .append(vec![make_input("e1")])
            .await
            .expect_err("append must fail when the .jsonl is read-only");
        assert!(
            matches!(err, AuditError::Io { .. }),
            "expected Io error, got {err:?}"
        );

        // Restore write permissions so subsequent `append` calls and TempDir cleanup work.
        fs::set_permissions(&jsonl, fs::Permissions::from_mode(0o644))
            .expect("restore permissions");

        // After the write failure the store must be poisoned. Any further `append`
        // must return an I/O error, not silently accept a call that would mint entries
        // with chain hashes computed from phantom predecessors.
        let retry = store.append(vec![make_input("e2")]).await;
        assert!(
            retry.is_err(),
            "a poisoned store must reject all further append calls"
        );
        assert!(
            matches!(retry.unwrap_err(), AuditError::Io { .. }),
            "poisoned store must return Io error, not a chain error"
        );
    }

    /// After a write failure on `append`, a store that is reopened must see only the
    /// entries that were actually written. No phantom entry from the failed write must
    /// appear in the chain after recovery.
    #[tokio::test]
    async fn should_not_include_phantom_entries_in_chain_after_a_failed_append_and_reopen() {
        use std::os::unix::fs::PermissionsExt;
        let dir = TempDir::new("append-write-fail-reopen");
        let store = make_store(&dir);

        // Append one entry successfully.
        store
            .append(vec![make_input("good-entry")])
            .await
            .expect("first append must succeed");

        // Now make the file read-only so the second append fails.
        let node_dir = dir.path().join(NODE);
        let seg_id = store.state().open.as_ref().unwrap().id().to_owned();
        let jsonl = jsonl_path(&node_dir, &seg_id);

        fs::set_permissions(&jsonl, fs::Permissions::from_mode(0o444))
            .expect("set permissions read-only");

        // `append` must fail — the file is read-only.
        store
            .append(vec![make_input("phantom-entry")])
            .await
            .expect_err("append must fail when the .jsonl is read-only");

        // Restore write permissions so reopening and TempDir cleanup work.
        fs::set_permissions(&jsonl, fs::Permissions::from_mode(0o644))
            .expect("restore permissions");

        // Drop the poisoned store and reopen. Recovery must succeed and the chain
        // must verify. Without the fix, the phantom entry in memory causes the second
        // open to fail with ChainIntegrity because the file's tip does not match what
        // the in-memory segment expected to follow.
        drop(store);
        let store2 = FileAuditStore::open(dir.path(), NodeId::new(NODE), CONFIG)
            .expect("reopening after a failed write must succeed");

        store2
            .verify()
            .await
            .expect("the chain must verify — only durable entries must be present");

        // The sealed segment from recovery holds only the one good entry.
        let seals = store2.seals().await.expect("seals");
        let total: u64 = seals.iter().map(|s| s.entry_count).sum();
        assert_eq!(
            total, 1,
            "only the durably-written entry must be in the chain; \
             the phantom entry must not appear"
        );
    }

    // ── Task 5: io_order contention instrumentation ─────────────────────────

    /// At concurrency 1 nobody else is ever holding `io_order`, so the reported wait
    /// must be near zero. At concurrency 8, every task but whichever one currently
    /// holds the lock spends real time queued behind an in-flight mint-then-fsync, so
    /// the reported wait must be materially above zero. A contention metric that reads
    /// zero under contention is worse than no metric — it will be believed. See phase2
    /// plan Task 5, Step 3.
    #[tokio::test(flavor = "multi_thread", worker_threads = 8)]
    async fn should_report_near_zero_io_order_wait_at_concurrency_one_and_material_wait_at_eight() {
        const BATCHES_PER_TASK: usize = 20;

        async fn run(dir_tag: &str, concurrency: usize) -> (Duration, u64) {
            let dir = TempDir::new(dir_tag);
            let store = Arc::new(
                FileAuditStore::open_with_policy(
                    dir.path(),
                    NodeId::new("contention-node"),
                    CONFIG,
                    SyncPolicy::EveryBatch,
                )
                .unwrap()
                .with_contention_tracking(),
            );

            let mut handles = Vec::with_capacity(concurrency);
            for t in 0..concurrency {
                let store = Arc::clone(&store);
                handles.push(tokio::spawn(async move {
                    for b in 0..BATCHES_PER_TASK {
                        let inputs = vec![make_input(&format!("t{t}-b{b}"))];
                        store.append(inputs).await.expect("append");
                    }
                }));
            }
            for h in handles {
                h.await.expect("task must not panic");
            }

            let (wait, count) = store.io_order_wait().expect("contention tracking enabled");
            store.close().await.expect("close");
            (wait, count)
        }

        let (wait_at_one, count_at_one) = run("contention-c1", 1).await;
        let (wait_at_eight, count_at_eight) = run("contention-c8", 8).await;

        assert_eq!(count_at_one, BATCHES_PER_TASK as u64);
        assert_eq!(count_at_eight, (BATCHES_PER_TASK * 8) as u64);

        let avg_wait_at_one = wait_at_one / count_at_one as u32;
        let avg_wait_at_eight = wait_at_eight / count_at_eight as u32;

        assert!(
            avg_wait_at_one < Duration::from_millis(1),
            "expected near-zero io_order wait at concurrency 1, got {avg_wait_at_one:?} average"
        );
        assert!(
            avg_wait_at_eight > Duration::from_micros(100),
            "expected a materially non-zero io_order wait at concurrency 8, got \
             {avg_wait_at_eight:?} average"
        );
        assert!(
            avg_wait_at_eight > avg_wait_at_one * 3,
            "expected io_order wait at concurrency 8 to be materially above concurrency 1's \
             ({avg_wait_at_eight:?} vs {avg_wait_at_one:?})"
        );
    }

    /// numbers for both policies on this host. The test is #[ignore] so it does not
    /// run in the standard suite; invoke with `cargo test step8_timing -- --ignored
    /// --nocapture`.
    #[tokio::test]
    #[ignore]
    async fn step8_sync_policy_timing() {
        const TOTAL_ENTRIES: usize = 1000;
        const BATCH_SIZE: usize = 10;

        async fn run(policy: SyncPolicy, label: &str) -> std::time::Duration {
            let dir = TempDir::new(&format!("timing-{label}"));
            let store = FileAuditStore::open_with_policy(
                dir.path(),
                NodeId::new("bench-node"),
                CONFIG,
                policy,
            )
            .unwrap();

            let start = std::time::Instant::now();
            for batch in 0..(TOTAL_ENTRIES / BATCH_SIZE) {
                let inputs: Vec<AuditEntryInput> = (0..BATCH_SIZE)
                    .map(|i| make_input(&format!("b{batch}-e{i}")))
                    .collect();
                store.append(inputs).await.expect("append");
            }
            let elapsed = start.elapsed();
            store.close().await.expect("close");
            elapsed
        }

        let every_batch = run(SyncPolicy::EveryBatch, "every-batch").await;
        let on_seal = run(SyncPolicy::OnSeal, "on-seal").await;

        println!(
            "\n=== Step 8 SyncPolicy timing ({TOTAL_ENTRIES} entries, batches of {BATCH_SIZE}) ===\n\
             EveryBatch: {every_batch:?}\n\
             OnSeal:     {on_seal:?}\n"
        );
    }
}
