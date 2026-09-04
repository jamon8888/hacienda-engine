//! Durable, fsync-backed implementation of [`ReviewStore`].
//!
//! # File format
//!
//! A single JSON-lines file — one event per line — stores the full history of every
//! review item. Three event kinds cover the lifecycle:
//!
//! ```text
//! {"kind":"Submitted","item":{…}}
//! {"kind":"Assigned","id":"…","reviewer":"…","assigned_at":"…"}
//! {"kind":"Decided","id":"…","decision":"approve","reviewer":"…","comment":"…","decided_at":"…"}
//! ```
//!
//! On `open`, the log is replayed from top to bottom to reconstruct current state. This
//! means a restart is as cheap as re-reading the file — no compaction, no snapshot.
//!
//! # Truncated trailing line
//!
//! The file format is chosen precisely because a crash mid-append leaves a partial JSON
//! object at the end of the file. Refusing to open in that case would be the wrong
//! response to the normal failure mode. Instead:
//!
//! - The **trailing line** is dropped with a [`tracing::warn!`] if it cannot be parsed.
//! - A malformed line **anywhere else in the file** is an [`ReviewError::Json`] error.
//!   Something else is wrong; appending further events would compound unknown corruption.
//!
//! # Durability
//!
//! `File::flush` alone does not survive power loss. Every append calls `File::sync_data()`
//! to push the write through to stable storage before returning `Ok`.
//!
//! # Locking discipline — why there are two locks
//!
//! Every mutating method has two critical sections: mutate in-memory state under a
//! `std::sync::Mutex`, then write to disk under `tokio::task::spawn_blocking`. Two
//! concurrent callers can therefore mutate in order (A, B) and write in order (B, A).
//! In memory the result is correct — mutation was serialised. On disk, a `Decided` event
//! can land before its `Assigned` event. Replay then reconstructs a different outcome
//! than the live store had.
//!
//! `io_order: tokio::sync::Mutex<()>` is held across the entire mutate-then-write
//! sequence, making on-disk order equal in-memory mutation order. It must span an
//! `.await` (the `spawn_blocking` call), so it must be a tokio mutex rather than a
//! `std` mutex. This is not a workaround for the `!Send` error a held `std` guard would
//! produce — that error is correct and must not be suppressed.
//!
//! Lock order is always `io_order` first, then `state`. Never the reverse.
//!
//! [`ReviewStore`]: super::store::ReviewStore

use async_trait::async_trait;
use std::collections::HashMap;
use std::fs::{self, File, OpenOptions};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

use crate::review::error::ReviewError;
use crate::review::store::ReviewStore;
use crate::review::types::{QueueStats, ReviewDecision, ReviewQueueItem, ReviewStatus};
use crate::tenancy::TenantId;

// ── Event types ───────────────────────────────────────────────────────────────

/// One entry in the append-only event log.
///
/// The log is the authoritative record. In-memory state is a projection of the log
/// rebuilt on every `open`. Adding a new event kind here requires a matching arm in
/// `replay_events` or the store will refuse to open files that contain it.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind")]
enum ReviewEvent {
    /// A new item was submitted. The full item is stored so replay can reconstruct it
    /// without knowing anything about the submission logic.
    Submitted { item: ReviewQueueItem },

    /// An item moved from `Pending` to `InReview`.
    Assigned {
        id: String,
        reviewer: String,
        assigned_at: String,
    },

    /// A decision was recorded on an item.
    Decided {
        id: String,
        decision: ReviewDecision,
        reviewer: String,
        comment: String,
        decided_at: String,
    },
}

// ── FileState ─────────────────────────────────────────────────────────────────

/// All mutable in-memory state for a `FileReviewStore`, behind one lock.
///
/// Using a `HashMap` keyed by id gives O(1) lookup in `assign` and `decide`, which
/// matters for the CAS hot path. `Vec<String>` preserves insertion order for `list`.
#[derive(Debug, Default)]
struct FileState {
    /// Items keyed by id for O(1) mutation in assign/decide.
    by_id: HashMap<String, ReviewQueueItem>,
    /// Insertion order, for deterministic `list` output.
    order: Vec<String>,
}

impl FileState {
    fn get_mut(&mut self, id: &str) -> Option<&mut ReviewQueueItem> {
        self.by_id.get_mut(id)
    }

    /// Same as [`Self::get_mut`], but an id belonging to a different tenant is treated
    /// exactly like an id that does not exist — `None` either way. This is what makes
    /// the not-found-not-forbidden property (D-S1b-1) fall out of the lookup itself
    /// rather than needing a second branch at every call site to enforce it.
    fn get_mut_for_tenant(&mut self, tenant: &TenantId, id: &str) -> Option<&mut ReviewQueueItem> {
        self.by_id.get_mut(id).filter(|item| item.tenant_id == tenant.as_str())
    }

    fn insert(&mut self, item: ReviewQueueItem) {
        self.order.push(item.id.clone());
        self.by_id.insert(item.id.clone(), item);
    }

    fn items_in_order(&self) -> Vec<ReviewQueueItem> {
        self.order
            .iter()
            .filter_map(|id| self.by_id.get(id))
            .cloned()
            .collect()
    }
}

// ── FileReviewStore ───────────────────────────────────────────────────────────

/// A durable [`ReviewStore`] that persists events to a JSON-lines file.
///
/// See module documentation for the file format, truncation handling, durability
/// guarantees, and locking discipline.
///
/// # Recovery
///
/// `open` reads every line of the event log and replays them in order into a fresh
/// `FileState`. A partial trailing line (the normal result of a crash mid-append)
/// is dropped with a warning. A malformed line anywhere else in the file is a hard
/// error — the corruption is unexpected and continuing would make it worse.
///
/// # CAS semantics
///
/// `assign` and `decide` are compare-and-swap operations that must not be split into
/// read-then-write. Both hold `io_order` and then the `state` lock across the full
/// check-and-mutate sequence, exactly as [`InMemoryReviewStore`] does.
///
/// # Write-failure poisoning (Design Decision: poison on write failure)
///
/// Every mutating method applies a two-step sequence under `io_order`:
///   1. Mutate in-memory state under the `state` lock, then drop the guard.
///   2. Write to disk on a blocking thread.
///
/// If the disk write fails, in-memory state already reflects the mutation while the
/// disk does not. The options are:
///
/// - **Roll back** the in-memory mutation. Cheap, but readers hold only the `state`
///   lock (not `io_order`), so they can observe the transient value between steps 1
///   and 2 — before the rollback executes. A reader who sees a phantom `Approved`
///   decision and hands it to the caller is served stale truth.
///
/// - **Write first, mutate after**. Cannot be applied here because the event bytes
///   are serialised inside the `state` lock after the mutation; separating them would
///   require restructuring the CAS checks.
///
/// - **Poison the store** on write failure. After a disk write returns an error, set
///   an atomic flag. Every subsequent call — read or write — returns an `Io` error
///   immediately. The transient read window (between mutation and poisoning) is the
///   same size as the rollback window, but crucially: any _retry_ of the failed
///   operation returns `Io`, never `AlreadyDecided`. The caller knows the store is
///   unhealthy. Reopen replays from disk, which is the source of truth and was never
///   updated, giving a clean slate.
///
/// Poison is chosen because it closes the retry trap described in the issue: after
/// `decide` fails with `Io`, a caller who retries must not receive `AlreadyDecided`
/// (which would imply a durable decision that never reached disk). Poison makes that
/// impossible. Rollback would not: the rollback may itself fail, or may not have run
/// yet when the retry arrives.
///
/// [`InMemoryReviewStore`]: crate::review::store::InMemoryReviewStore
/// [`ReviewStore`]: crate::review::store::ReviewStore
#[derive(Debug)]
/// FileReviewStore struct
pub struct FileReviewStore {
    /// Serialises the mutate-then-write sequence so on-disk event order matches
    /// in-memory mutation order. See module docs "why there are two locks".
    io_order: tokio::sync::Mutex<()>,
    state: Mutex<FileState>,
    log_path: PathBuf,
    /// Set to `true` after any disk write failure. Once poisoned, all subsequent
    /// operations return an `Io` error immediately. The caller must reopen the store
    /// (which replays from disk) to recover. See struct-level docs for rationale.
    poisoned: AtomicBool,
}

impl FileReviewStore {
    /// Open (or create) a durable review store at `path`.
    ///
    /// If the file already exists its events are replayed to reconstruct state. An
    /// unterminated trailing record is *removed from the file* with a warning; a
    /// malformed line that is properly terminated is a hard error.
    ///
    /// Removing rather than merely skipping the partial record is what makes the log
    /// writable again. `append` writes at EOF, so a fragment left in place would be
    /// welded to the next event on the same physical line — and that welded line, being
    /// last, would itself look truncated. Every subsequent write would be accepted,
    /// fsynced, and silently discarded. Truncation happens once, here, under the
    /// caller's control.
    ///
    /// # Errors
    ///
    /// - [`ReviewError::Io`] — file creation, read, or truncation failed.
    /// - [`ReviewError::Json`] — a terminated line is not valid JSON.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, ReviewError> {
        let path = path.as_ref().to_path_buf();

        // Create the parent directory if it does not exist yet.
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|source| ReviewError::Io {
                path: parent.display().to_string(),
                source,
            })?;
        }

        // If the file does not exist, create it empty and return a fresh store.
        if !path.exists() {
            File::create(&path).map_err(|source| ReviewError::Io {
                path: path.display().to_string(),
                source,
            })?;
            return Ok(Self {
                io_order: tokio::sync::Mutex::new(()),
                state: Mutex::new(FileState::default()),
                log_path: path,
                poisoned: AtomicBool::new(false),
            });
        }

        // Read as bytes, not as a string: a crash can split a multi-byte UTF-8
        // character, and `read_to_string` would refuse to open the file at all rather
        // than let us discard the partial record.
        let raw = fs::read(&path).map_err(|source| ReviewError::Io {
            path: path.display().to_string(),
            source,
        })?;

        let complete = truncate_to_last_complete_record(&path, &raw)?;
        let contents = std::str::from_utf8(complete).map_err(|err| ReviewError::Io {
            path: path.display().to_string(),
            source: std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("review event log is not valid UTF-8: {err}"),
            ),
        })?;

        let state = replay_events(contents)?;

        Ok(Self {
            io_order: tokio::sync::Mutex::new(()),
            state: Mutex::new(state),
            log_path: path,
            poisoned: AtomicBool::new(false),
        })
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
    fn check_not_poisoned(&self) -> Result<(), ReviewError> {
        if self.poisoned.load(Ordering::Acquire) {
            return Err(ReviewError::Io {
                path: self.log_path.display().to_string(),
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
    fn poison(&self, error: ReviewError) -> ReviewError {
        self.poisoned.store(true, Ordering::Release);
        tracing::error!(
            path = %self.log_path.display(),
            "review store poisoned after a write failure; reopen required"
        );
        error
    }
}

// ── ReviewStore impl ──────────────────────────────────────────────────────────

#[async_trait]
impl ReviewStore for FileReviewStore {
    async fn submit(
        &self,
        tenant: &TenantId,
        mut item: ReviewQueueItem,
    ) -> Result<ReviewQueueItem, ReviewError> {
        item.tenant_id = tenant.to_string();
        self.check_not_poisoned()?;

        // Acquire io_order first, then the state lock — always this order.
        let _io = self.io_order.lock().await;

        let event = ReviewEvent::Submitted { item: item.clone() };
        let bytes = {
            let mut state = self.state();
            // `ReviewQueue` generates uuid v4 ids before calling here, so duplicate
            // ids cannot occur in normal operation. Inserting unconditionally means
            // a bug in the caller replaces the existing item rather than silently
            // creating two items with the same id, which would be worse.
            state.insert(item.clone());
            // Serialise inside the lock so the byte layout is determined before
            // we drop the guard. The write happens outside the lock.
            serde_json::to_vec(&event)?
            // ── state guard drops here; `_io` is still held ───────────────────
        };

        let log_path = self.log_path.clone();
        let result = tokio::task::spawn_blocking(move || append_bytes_and_sync(&log_path, &bytes))
            .await
            .map_err(|join_err| ReviewError::Io {
                path: self.log_path.display().to_string(),
                source: std::io::Error::other(join_err.to_string()),
            })?;

        if let Err(err) = result {
            return Err(self.poison(err));
        }

        Ok(item)
    }

    async fn assign(
        &self,
        tenant: &TenantId,
        id: &str,
        reviewer: &str,
    ) -> Result<ReviewQueueItem, ReviewError> {
        self.check_not_poisoned()?;

        // The entire check-and-mutate sequence runs under io_order + state lock. This
        // is what preserves the CAS property: two concurrent callers cannot both see
        // `Pending` and both proceed, because the second one blocks on io_order until
        // the first has written the Assigned event to disk. Splitting this into a
        // read under the state lock, dropping the lock, and writing to disk would open
        // a window where two callers both observe Pending and both win.
        let _io = self.io_order.lock().await;

        let assigned_at = chrono::Utc::now().to_rfc3339();

        let (updated_item, bytes) = {
            let mut state = self.state();
            let item = state
                .get_mut_for_tenant(tenant, id)
                .ok_or_else(|| ReviewError::NotFound(id.to_string()))?;

            if item.status != ReviewStatus::Pending {
                return Err(ReviewError::InvalidTransition {
                    from: item.status.to_string(),
                    to: ReviewStatus::InReview.to_string(),
                });
            }

            item.assigned_reviewer = Some(reviewer.to_string());
            item.status = ReviewStatus::InReview;
            let updated = item.clone();

            let event = ReviewEvent::Assigned {
                id: id.to_string(),
                reviewer: reviewer.to_string(),
                assigned_at: assigned_at.clone(),
            };
            let bytes = serde_json::to_vec(&event)?;
            (updated, bytes)
            // ── state guard drops here ─────────────────────────────────────────
        };

        let log_path = self.log_path.clone();
        let result = tokio::task::spawn_blocking(move || append_bytes_and_sync(&log_path, &bytes))
            .await
            .map_err(|join_err| ReviewError::Io {
                path: self.log_path.display().to_string(),
                source: std::io::Error::other(join_err.to_string()),
            })?;

        if let Err(err) = result {
            return Err(self.poison(err));
        }

        Ok(updated_item)
    }

    async fn decide(
        &self,
        tenant: &TenantId,
        id: &str,
        decision: ReviewDecision,
        reviewer: &str,
        comment: &str,
    ) -> Result<ReviewQueueItem, ReviewError> {
        self.check_not_poisoned()?;

        // Same single-acquisition discipline as `assign`. See the module docs and the
        // struct-level "why there are two locks" section.
        let _io = self.io_order.lock().await;

        let decided_at = chrono::Utc::now().to_rfc3339();

        let (updated_item, bytes) = {
            let mut state = self.state();
            let item = state
                .get_mut_for_tenant(tenant, id)
                .ok_or_else(|| ReviewError::NotFound(id.to_string()))?;

            if item.decision.is_some() {
                return Err(ReviewError::AlreadyDecided(id.to_string()));
            }

            item.decision = Some(decision);
            item.status = match decision {
                ReviewDecision::Approve => ReviewStatus::Approved,
                ReviewDecision::Reject => ReviewStatus::Rejected,
                ReviewDecision::Modify => ReviewStatus::Modified,
            };
            item.decided_by = Some(reviewer.to_string());
            item.decided_at = Some(decided_at.clone());
            item.comment = Some(comment.to_string());
            let updated = item.clone();

            let event = ReviewEvent::Decided {
                id: id.to_string(),
                decision,
                reviewer: reviewer.to_string(),
                comment: comment.to_string(),
                decided_at,
            };
            let bytes = serde_json::to_vec(&event)?;
            (updated, bytes)
            // ── state guard drops here ─────────────────────────────────────────
        };

        let log_path = self.log_path.clone();
        let result = tokio::task::spawn_blocking(move || append_bytes_and_sync(&log_path, &bytes))
            .await
            .map_err(|join_err| ReviewError::Io {
                path: self.log_path.display().to_string(),
                source: std::io::Error::other(join_err.to_string()),
            })?;

        if let Err(err) = result {
            return Err(self.poison(err));
        }

        Ok(updated_item)
    }

    async fn list(
        &self,
        tenant: &TenantId,
        filter: Option<ReviewStatus>,
    ) -> Result<Vec<ReviewQueueItem>, ReviewError> {
        self.check_not_poisoned()?;
        let state = self.state();
        let mine = state
            .items_in_order()
            .into_iter()
            .filter(|i| i.tenant_id == tenant.as_str());
        let result = match filter {
            Some(status) => mine.filter(|i| i.status == status).collect(),
            None => mine.collect(),
        };
        Ok(result)
    }

    async fn get(
        &self,
        tenant: &TenantId,
        id: &str,
    ) -> Result<Option<ReviewQueueItem>, ReviewError> {
        self.check_not_poisoned()?;
        Ok(self
            .state()
            .by_id
            .get(id)
            .filter(|item| item.tenant_id == tenant.as_str())
            .cloned())
    }

    async fn stats(&self, tenant: &TenantId) -> Result<QueueStats, ReviewError> {
        self.check_not_poisoned()?;
        let state = self.state();
        let all: Vec<ReviewQueueItem> = state
            .items_in_order()
            .into_iter()
            .filter(|i| i.tenant_id == tenant.as_str())
            .collect();
        let count = |status: ReviewStatus| all.iter().filter(|i| i.status == status).count();
        Ok(QueueStats {
            total: all.len(),
            pending: count(ReviewStatus::Pending),
            in_review: count(ReviewStatus::InReview),
            approved: count(ReviewStatus::Approved),
            rejected: count(ReviewStatus::Rejected),
            modified: count(ReviewStatus::Modified),
        })
    }
}

// ── Replay ────────────────────────────────────────────────────────────────────

/// Cut any unterminated trailing bytes off the log and return the complete prefix.
///
/// The newline is the record terminator, so "was this record fully written?" is answered
/// by the presence of the terminator, not by whether the bytes happen to parse. That
/// distinction matters: `append_bytes_and_sync` issues the payload and the newline as two
/// writes, so a crash between them leaves a *parseable* record that is nonetheless
/// incomplete. Treating it as complete would then make the file's last line unterminated
/// forever.
///
/// The file itself is shortened, not merely the in-memory copy — see [`FileReviewStore::open`].
///
/// # Errors
///
/// [`ReviewError::Io`] if the file cannot be opened for truncation or `set_len` fails.
fn truncate_to_last_complete_record<'a>(
    path: &Path,
    raw: &'a [u8],
) -> Result<&'a [u8], ReviewError> {
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
        "review event log ended mid-record; discarding the unterminated tail so the log stays appendable"
    );

    let io_error = |source| ReviewError::Io {
        path: path.display().to_string(),
        source,
    };
    let file = OpenOptions::new()
        .write(true)
        .open(path)
        .map_err(io_error)?;
    file.set_len(keep as u64).map_err(io_error)?;
    // Persist the shortened length. Without this the next crash resurrects the tail.
    file.sync_all().map_err(io_error)?;

    Ok(&raw[..keep])
}

/// Replay every line of `contents` into a fresh `FileState`.
///
/// Every line reaching this function is terminated, and therefore was fully written —
/// [`truncate_to_last_complete_record`] has already removed any partial tail. So a line
/// that does not parse is real corruption, not a crash artefact, and is returned as
/// [`ReviewError::Json`] rather than skipped. Skipping it would silently drop a decision
/// a human actually made.
fn replay_events(contents: &str) -> Result<FileState, ReviewError> {
    let mut state = FileState::default();

    for line in contents.lines().filter(|l| !l.trim().is_empty()) {
        apply_event(&mut state, serde_json::from_str(line)?);
    }

    Ok(state)
}

/// Apply one event to the in-memory state projection.
///
/// This function encodes the same logic as the mutating methods on `FileReviewStore`
/// but without any I/O or error propagation — replay must be infallible for events that
/// were already validated when written.
fn apply_event(state: &mut FileState, event: ReviewEvent) {
    match event {
        ReviewEvent::Submitted { item } => {
            state.insert(item);
        }
        ReviewEvent::Assigned {
            id,
            reviewer,
            assigned_at: _,
        } => {
            if let Some(item) = state.get_mut(&id) {
                item.assigned_reviewer = Some(reviewer);
                item.status = ReviewStatus::InReview;
            }
            // An Assigned event for an unknown id should not happen in a
            // well-formed log, but silently ignoring it during replay is safer
            // than refusing to open and losing all good events that follow.
        }
        ReviewEvent::Decided {
            id,
            decision,
            reviewer,
            comment,
            decided_at,
        } => {
            if let Some(item) = state.get_mut(&id) {
                item.decision = Some(decision);
                item.status = match decision {
                    ReviewDecision::Approve => ReviewStatus::Approved,
                    ReviewDecision::Reject => ReviewStatus::Rejected,
                    ReviewDecision::Modify => ReviewStatus::Modified,
                };
                item.decided_by = Some(reviewer);
                item.decided_at = Some(decided_at);
                item.comment = Some(comment);
            }
        }
    }
}

// ── File helpers ──────────────────────────────────────────────────────────────

/// Append `bytes` followed by a newline to `path`, then `sync_data`.
///
/// Called from `spawn_blocking`; must not be called from async context directly.
fn append_bytes_and_sync(path: &Path, bytes: &[u8]) -> Result<(), ReviewError> {
    let file = OpenOptions::new()
        .append(true)
        .open(path)
        .map_err(|source| ReviewError::Io {
            path: path.display().to_string(),
            source,
        })?;
    let mut writer = BufWriter::new(&file);
    writer.write_all(bytes).map_err(|source| ReviewError::Io {
        path: path.display().to_string(),
        source,
    })?;
    // `serde_json::to_vec` produces no trailing newline. Add one so each event
    // occupies exactly one line, which is what `replay_events` and the truncation
    // detection both rely on.
    writer.write_all(b"\n").map_err(|source| ReviewError::Io {
        path: path.display().to_string(),
        source,
    })?;
    writer.flush().map_err(|source| ReviewError::Io {
        path: path.display().to_string(),
        source,
    })?;
    file.sync_data().map_err(|source| ReviewError::Io {
        path: path.display().to_string(),
        source,
    })?;
    Ok(())
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::review::types::{Priority, ReviewDecision, ReviewQueueItem};
    use std::sync::Arc;

    // ── TempDir RAII helper ───────────────────────────────────────────────────
    // Copied from audit/store_file.rs. Both modules need isolation; the helper is
    // not worth making pub(crate) just to share six lines.

    struct TempDir(PathBuf);

    impl TempDir {
        fn new(tag: &str) -> Self {
            let path = std::env::temp_dir().join(format!(
                "hacienda-file-review-{tag}-{}",
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

    /// The tenant used by every test that doesn't itself care about tenant scoping.
    fn t() -> TenantId {
        TenantId::new("tenant-a")
    }

    fn make_item(id: &str) -> ReviewQueueItem {
        ReviewQueueItem {
            id: id.to_string(),
            text_snippet: "John Smith".to_string(),
            category: "PersonName".to_string(),
            start: 0,
            end: 10,
            confidence: 0.75,
            source: "regex".to_string(),
            status: ReviewStatus::Pending,
            priority: Priority::Normal,
            assigned_reviewer: None,
            created_at: chrono::Utc::now().to_rfc3339(),
            deadline: None,
            decision: None,
            decided_by: None,
            decided_at: None,
            comment: None,
            tenant_id: t().to_string(),
        }
    }

    fn log_path(dir: &TempDir) -> PathBuf {
        dir.path().join("review.jsonl")
    }

    fn open_store(dir: &TempDir) -> FileReviewStore {
        FileReviewStore::open(log_path(dir)).expect("store must open")
    }

    // ── Plan-specified tests ──────────────────────────────────────────────────

    /// After a simulated crash (drop without explicit close), reopening the store
    /// must replay every decision exactly as it was in the live store.
    #[tokio::test]
    async fn should_replay_every_decision_after_a_restart() {
        let dir = TempDir::new("replay-decisions");

        let item_id = {
            let store = open_store(&dir);
            let item = store
                .submit(&t(), make_item("item-1"))
                .await
                .expect("submit must succeed");
            store
                .assign(&t(), &item.id, "alice")
                .await
                .expect("assign must succeed");
            store
                .decide(
                    &t(),
                    &item.id,
                    ReviewDecision::Approve,
                    "alice",
                    "looks fine",
                )
                .await
                .expect("decide must succeed");
            item.id
        };
        // Drop = crash (no explicit close needed for an append-only log).

        let store2 = open_store(&dir);
        let replayed = store2
            .get(&t(), &item_id)
            .await
            .expect("get must not error")
            .expect("item must be present after restart");

        assert_eq!(replayed.status, ReviewStatus::Approved);
        assert_eq!(replayed.decision, Some(ReviewDecision::Approve));
        assert_eq!(replayed.decided_by.as_deref(), Some("alice"));
        assert_eq!(replayed.comment.as_deref(), Some("looks fine"));
    }

    /// Every status transition — Pending → InReview → Approved — must survive a
    /// restart between each step.
    #[tokio::test]
    async fn should_preserve_status_transitions_across_a_restart() {
        let dir = TempDir::new("transitions");

        // Step 1: submit.
        {
            let store = open_store(&dir);
            store
                .submit(&t(), make_item("item-2"))
                .await
                .expect("submit");
        }

        // Step 2: assign — opens fresh store from disk.
        {
            let store = open_store(&dir);
            let item = store
                .get(&t(), "item-2")
                .await
                .expect("get")
                .expect("must be present");
            assert_eq!(item.status, ReviewStatus::Pending, "must replay as Pending");
            store.assign(&t(), "item-2", "bob").await.expect("assign");
        }

        // Step 3: decide — opens fresh store again.
        {
            let store = open_store(&dir);
            let item = store
                .get(&t(), "item-2")
                .await
                .expect("get")
                .expect("must be present");
            assert_eq!(
                item.status,
                ReviewStatus::InReview,
                "must replay as InReview"
            );
            store
                .decide(&t(), "item-2", ReviewDecision::Reject, "bob", "not PII")
                .await
                .expect("decide");
        }

        // Step 4: final state check.
        {
            let store = open_store(&dir);
            let item = store
                .get(&t(), "item-2")
                .await
                .expect("get")
                .expect("must be present");
            assert_eq!(item.status, ReviewStatus::Rejected);
            assert_eq!(item.decision, Some(ReviewDecision::Reject));
        }
    }

    /// A truncated trailing line — the normal result of a crash mid-append — must
    /// be dropped with a warning and the rest of the log must replay cleanly.
    #[tokio::test]
    async fn should_drop_a_truncated_trailing_line_and_keep_the_rest() {
        let dir = TempDir::new("truncated");

        // Write two complete items.
        {
            let store = open_store(&dir);
            store
                .submit(&t(), make_item("item-3"))
                .await
                .expect("submit 3");
            store
                .submit(&t(), make_item("item-4"))
                .await
                .expect("submit 4");
        }

        // Append a partial (truncated) JSON line to the log file.
        {
            let path = log_path(&dir);
            let mut file = OpenOptions::new()
                .append(true)
                .open(&path)
                .expect("open for append");
            // Partial JSON — no closing brace, simulating a crash mid-write.
            file.write_all(b"{\"kind\":\"Submitted\",\"item\":{\"id\":\"item-x")
                .expect("write truncated");
            file.sync_data().expect("sync");
        }

        // The store must open successfully and replay both complete items.
        let store = open_store(&dir);
        let items = store
            .list(&t(), None)
            .await
            .expect("list must not error despite truncated line");

        assert_eq!(items.len(), 2, "the two complete items must be present");
        assert!(
            items.iter().any(|i| i.id == "item-3"),
            "item-3 must be replayed"
        );
        assert!(
            items.iter().any(|i| i.id == "item-4"),
            "item-4 must be replayed"
        );
    }

    /// Recovering from a truncated trailing line must leave the log *writable*, not just
    /// readable.
    ///
    /// This is the case the previous test misses. Dropping the bad line at replay time
    /// fixes the read, but the fragment is still sitting at the end of the file with no
    /// newline after it. The next `append` lands on the same physical line, welding the
    /// fragment to a valid event — and because that welded line is still the *last* line,
    /// replay drops it as "truncated" too. Every write after a single crash is then
    /// accepted, fsynced, and silently discarded, forever, with no error anywhere.
    ///
    /// Recovery must therefore truncate the file back to the last complete line.
    #[tokio::test]
    async fn should_accept_writes_after_recovering_from_a_truncated_line() {
        let dir = TempDir::new("truncated-then-write");

        {
            let store = open_store(&dir);
            store
                .submit(&t(), make_item("item-a"))
                .await
                .expect("submit a");
        }

        // Simulate a crash part-way through appending a second event.
        {
            let path = log_path(&dir);
            let mut file = OpenOptions::new()
                .append(true)
                .open(&path)
                .expect("open for append");
            file.write_all(b"{\"kind\":\"Submitted\",\"item\":{\"id\":\"item-lost")
                .expect("write truncated");
            file.sync_data().expect("sync");
        }

        // Recover and write a new item.
        {
            let store = open_store(&dir);
            store
                .submit(&t(), make_item("item-b"))
                .await
                .expect("submit b");
        }

        // The new item must survive one more restart.
        let store = open_store(&dir);
        let items = store
            .list(&t(), None)
            .await
            .expect("list after second restart");
        let ids: Vec<&str> = items.iter().map(|i| i.id.as_str()).collect();
        assert!(
            ids.contains(&"item-b"),
            "an item written after recovery must be durable; got {ids:?}"
        );
        assert_eq!(items.len(), 2, "item-a and item-b, not the lost fragment");
    }

    /// After a restart, the CAS semantics of `assign` must still hold: an assignment
    /// that lost the race before the crash must still lose after recovery.
    #[tokio::test]
    async fn should_reject_an_assignment_that_lost_the_race_after_a_restart() {
        let dir = TempDir::new("cas-restart");

        // Submit and assign before the crash.
        {
            let store = open_store(&dir);
            store
                .submit(&t(), make_item("item-5"))
                .await
                .expect("submit");
            store.assign(&t(), "item-5", "alice").await.expect("assign");
        }

        // After restart, a second assign must fail — the item is InReview, not Pending.
        let store2 = open_store(&dir);
        let result = store2.assign(&t(), "item-5", "bob").await;
        assert!(
            result.is_err(),
            "second assign must fail after restart — item is no longer Pending"
        );
        match result.unwrap_err() {
            ReviewError::InvalidTransition { from, to } => {
                assert_eq!(from, "in_review");
                assert_eq!(to, "in_review");
            }
            other => panic!("expected InvalidTransition, got {other:?}"),
        }
    }

    // ── Write-failure / poison tests ─────────────────────────────────────────

    /// After a write failure on `decide`, the store must be poisoned: all subsequent
    /// operations must return an I/O error. A retry must NOT return `AlreadyDecided`,
    /// which would only occur if the in-memory mutation were kept while the disk write
    /// was dropped — a phantom decision that would disappear on restart.
    ///
    /// Fault injection: make the log file read-only immediately after submit+assign so
    /// that the `decide` write fails with EACCES. Uses `#[cfg(test)] std::fs::set_permissions`
    /// — no production code is modified.
    #[tokio::test]
    async fn should_poison_store_and_not_expose_phantom_state_after_a_failed_decide_write() {
        use std::os::unix::fs::PermissionsExt;
        let dir = TempDir::new("decide-write-fail");
        let store = open_store(&dir);

        let item = store
            .submit(&t(), make_item("item-x"))
            .await
            .expect("submit must succeed");
        store
            .assign(&t(), &item.id, "alice")
            .await
            .expect("assign must succeed");

        // Make the log file read-only so the next write fails.
        let path = log_path(&dir);
        fs::set_permissions(&path, fs::Permissions::from_mode(0o444))
            .expect("set permissions read-only");

        // `decide` must fail — the disk write is rejected.
        let err = store
            .decide(&t(), &item.id, ReviewDecision::Approve, "alice", "ok")
            .await
            .expect_err("decide must fail when the log file is read-only");
        assert!(
            matches!(err, ReviewError::Io { .. }),
            "expected Io error, got {err:?}"
        );

        // Restore write permissions so TempDir cleanup can remove the file.
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).expect("restore permissions");

        // After the write failure the store must be poisoned. Any further operation
        // — including a retry of `decide` — must return an I/O error, NOT
        // `AlreadyDecided`. `AlreadyDecided` would indicate that the phantom in-memory
        // mutation is still visible and the caller's retry is being misdirected.
        let retry = store
            .decide(&t(), &item.id, ReviewDecision::Approve, "alice", "ok")
            .await;
        assert!(
            retry.is_err(),
            "a poisoned store must reject all further operations"
        );
        assert!(
            !matches!(retry.unwrap_err(), ReviewError::AlreadyDecided(_)),
            "retry must not return AlreadyDecided — that would be a phantom in-memory decision"
        );
    }

    /// A restart after a failed `decide` write must show the item in its pre-failure
    /// state, not as decided. The disk is the source of truth.
    #[tokio::test]
    async fn should_not_show_phantom_decision_after_a_failed_write_and_restart() {
        use std::os::unix::fs::PermissionsExt;
        let dir = TempDir::new("decide-write-fail-restart");
        let store = open_store(&dir);

        let item = store
            .submit(&t(), make_item("item-y"))
            .await
            .expect("submit must succeed");
        store
            .assign(&t(), &item.id, "alice")
            .await
            .expect("assign must succeed");

        // Make the log file read-only.
        let path = log_path(&dir);
        fs::set_permissions(&path, fs::Permissions::from_mode(0o444))
            .expect("set permissions read-only");

        // `decide` must fail.
        store
            .decide(&t(), &item.id, ReviewDecision::Approve, "alice", "ok")
            .await
            .expect_err("decide must fail when the log file is read-only");

        // Restore write permissions so the store can be reopened and TempDir can clean up.
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).expect("restore permissions");

        // Reopen from disk. The decision must NOT appear — it was never written.
        drop(store);
        let store2 = open_store(&dir);
        let replayed = store2
            .get(&t(), &item.id)
            .await
            .expect("get must not error")
            .expect("item must still be present");

        assert_eq!(
            replayed.status,
            ReviewStatus::InReview,
            "item must be InReview on disk (decision was never written), got {:?}",
            replayed.status
        );
        assert!(
            replayed.decision.is_none(),
            "a decision that was never written to disk must not appear after restart"
        );
    }

    // ── Write-ordering test ───────────────────────────────────────────────────

    /// Many concurrent `assign`/`decide` attempts against the same item must produce
    /// exactly one winner, and reopening from disk must report the same winner as the
    /// live store.
    ///
    /// Without `io_order`, a `Decided` event can reach the file before the `Assigned`
    /// event that preceded it in memory. Replay then reconstructs a *different* outcome
    /// from the one the live store served: `Assigned` is applied last and overwrites the
    /// decision with `InReview`, so a decision a human actually made disappears on
    /// restart. Nothing logs an error — the events are all there, just in the wrong order.
    ///
    /// Many independent items rather than one contended item: each item contributes an
    /// independent chance for the two writes to cross, so the race is exercised
    /// `ITEMS * RUNS` times instead of once per run. A single contended item makes the
    /// window so narrow that the test passes with `io_order` removed, which would make it
    /// decorative.
    ///
    /// Verified by mutation: commenting out all three `io_order` acquisitions fails this
    /// test on every run.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn should_write_concurrent_events_in_order_so_replay_matches_live_state() {
        const RUNS: usize = 3;
        const ITEMS: usize = 40;

        for run in 0..RUNS {
            let dir = TempDir::new(&format!("concurrent-order-{run}"));
            let log = log_path(&dir);
            let store = Arc::new(FileReviewStore::open(log.clone()).expect("open"));

            for i in 0..ITEMS {
                store
                    .submit(&t(), make_item(&format!("item-{i}")))
                    .await
                    .expect("submit");
            }

            // For each item, an assign and a decide race from two different tasks. They
            // touch the same item but neither blocks the other in memory: `decide` only
            // rejects an item that already carries a decision, so both succeed and both
            // write. That is exactly the pair whose file order must match its memory order.
            let mut handles = Vec::with_capacity(ITEMS * 2);
            for i in 0..ITEMS {
                let id = format!("item-{i}");

                let assigner = Arc::clone(&store);
                let assign_id = id.clone();
                handles.push(tokio::spawn(async move {
                    let _ = assigner.assign(&t(), &assign_id, "alice").await;
                }));

                let decider = Arc::clone(&store);
                handles.push(tokio::spawn(async move {
                    let _ = decider
                        .decide(&t(), &id, ReviewDecision::Approve, "bob", "ok")
                        .await;
                }));
            }
            for handle in handles {
                handle.await.expect("task must not panic");
            }

            let live = store.list(&t(), None).await.expect("live list");

            drop(store);
            let replayed = FileReviewStore::open(log)
                .expect("reopen after concurrent writes")
                .list(&t(), None)
                .await
                .expect("replayed list");

            assert_eq!(live.len(), replayed.len(), "run {run}: item count");
            for live_item in &live {
                let replayed_item = replayed
                    .iter()
                    .find(|i| i.id == live_item.id)
                    .expect("every live item must replay");
                assert_eq!(
                    (
                        live_item.status,
                        live_item.decision,
                        live_item.decided_by.as_deref()
                    ),
                    (
                        replayed_item.status,
                        replayed_item.decision,
                        replayed_item.decided_by.as_deref()
                    ),
                    "run {run}, {}: the reopened store must agree with the live one",
                    live_item.id
                );
            }
        }
    }
}
