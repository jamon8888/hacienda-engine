//! [`ReviewStore`] trait and the [`InMemoryReviewStore`] backend.
//!
//! The trait is the persistence seam that lets [`ReviewQueue`] swap between an in-memory
//! backend (default, same behaviour as before this task), a file backend (Task 5), and
//! eventually a Postgres backend (Phase 6) without any change to the call sites above it.
//! [`async_trait`] boxes the returned futures so that [`Arc<dyn ReviewStore>`] is object-safe
//! without native async-in-trait support.
//!
//! # Why `assign` and `decide` are atomic
//!
//! `assign` and `decide` are compare-and-swap operations: `assign` only moves an item from
//! `Pending` to `InReview`, and `decide` only records a decision when none exists yet. That
//! is what prevents two reviewers from both claiming the same item. If a store splits either
//! method into a `get` followed by a `put` — even briefly releasing the lock between them —
//! two concurrent callers can both observe the precondition satisfied and both proceed, with
//! one silently overwriting the other. Every implementation of this trait must hold one lock
//! (or equivalent) across the full check-then-mutate sequence.
//!
//! [`ReviewQueue`]: super::queue::ReviewQueue

use async_trait::async_trait;
use std::sync::{Arc, Mutex};

use crate::review::error::ReviewError;
use crate::review::types::{QueueStats, ReviewDecision, ReviewQueueItem, ReviewStatus};

// ── ReviewStore trait ─────────────────────────────────────────────────────────

/// A persistence backend for the human review queue.
///
/// All methods are `async` so that file and database backends can perform I/O without
/// blocking the async runtime. The in-memory backend does no I/O but uses the same
/// signatures, so callers never need to know which backend is present.
///
/// Implementations must be `Send + Sync` because the store lives behind an `Arc` shared
/// across tasks. Every method takes `&self` — interior mutability is the implementation's
/// responsibility, not the caller's.
///
/// # Object safety
///
/// `Arc<dyn ReviewStore>` is constructible. If you add a method with a type parameter or
/// that returns `Self`, you will break this property and every caller that holds a trait
/// object.
///
/// # Atomicity requirement on `assign` and `decide`
///
/// Both methods are compare-and-swap operations. **Never split them into read-then-write.**
/// The consequence of doing so is that two reviewers racing to claim the same item will both
/// pass the precondition check, and the second will silently overwrite the first. The in-memory
/// backend preserves this by holding the `Mutex` guard across the full check-and-mutate
/// sequence. A Postgres backend must use a `SELECT … FOR UPDATE` or an `UPDATE … WHERE status
/// = 'pending' RETURNING *`. Splitting the operation into `SELECT` + `UPDATE` — even within a
/// transaction at `READ COMMITTED` — reintroduces the race and will only appear under Phase 2's
/// concurrent workload, not in sequential tests.
#[async_trait]
pub trait ReviewStore: Send + Sync {
    /// Insert a pre-built item into the store.
    ///
    /// The item is constructed by [`ReviewQueue::submit`] which generates the id, priority,
    /// deadline, and timestamps before calling here. The store records what it receives.
    ///
    /// [`ReviewQueue::submit`]: super::queue::ReviewQueue::submit
    async fn submit(&self, item: ReviewQueueItem) -> Result<ReviewQueueItem, ReviewError>;

    /// Atomic compare-and-swap: move `id` from `Pending` to `InReview`.
    ///
    /// Succeeds only if the item's current status is `Pending`. Returns
    /// [`ReviewError::InvalidTransition`] if the status is anything else. The check and
    /// the mutation must happen under one lock acquisition — see the trait-level docs on
    /// atomicity. Splitting this into a `get` followed by a `put` is a correctness bug,
    /// not an optimisation.
    async fn assign(&self, id: &str, reviewer: &str) -> Result<ReviewQueueItem, ReviewError>;

    /// Atomic compare-and-swap: record a decision when none exists yet.
    ///
    /// Succeeds only if the item has no existing decision. Returns
    /// [`ReviewError::AlreadyDecided`] if a decision is already present. Like `assign`,
    /// this must be a single atomic check-and-mutate — see the trait-level docs.
    async fn decide(
        &self,
        id: &str,
        decision: ReviewDecision,
        reviewer: &str,
        comment: &str,
    ) -> Result<ReviewQueueItem, ReviewError>;

    /// Return all items, optionally restricted to a single status.
    async fn list(&self, filter: Option<ReviewStatus>)
        -> Result<Vec<ReviewQueueItem>, ReviewError>;

    /// Return a single item by id, or `None` if it does not exist.
    async fn get(&self, id: &str) -> Result<Option<ReviewQueueItem>, ReviewError>;

    /// Return counts of items in each status.
    async fn stats(&self) -> Result<QueueStats, ReviewError>;

    /// Release any resources the store holds open. Idempotent.
    ///
    /// The default is a no-op, which is the correct and complete implementation for both
    /// backends that exist today: [`InMemoryReviewStore`] holds nothing, and
    /// [`FileReviewStore`] opens, appends, `sync_data`s, and closes the log within each
    /// `append_bytes_and_sync` call, so there is no handle outstanding between operations.
    ///
    /// It exists so that [`HaciendaFacade::close`] has something to call. Without it the
    /// facade closes the audit store and silently skips the review store, and the first
    /// backend that does hold a resource — a Postgres connection pool in Phase 6 — would
    /// leak it with no call site to add the cleanup to. Adding the seam while it is free
    /// is cheaper than retrofitting it through every caller later.
    ///
    /// # Errors
    ///
    /// Whatever the implementation's cleanup returns. The default never errors.
    ///
    /// [`FileReviewStore`]: crate::review::store_file::FileReviewStore
    /// [`HaciendaFacade::close`]: crate::HaciendaFacade::close
    async fn close(&self) -> Result<(), ReviewError> {
        Ok(())
    }
}

// ── InMemoryReviewStore ───────────────────────────────────────────────────────

/// An in-memory implementation of [`ReviewStore`].
///
/// State is discarded on drop. Use this for testing and for short-lived processes where
/// review decisions do not need to survive a restart. Task 5's [`FileReviewStore`] provides
/// durability when it is needed.
///
/// # Locking discipline
///
/// Each method acquires one `Mutex` guard, does all of its work, and drops the guard before
/// returning — no guard crosses an `.await` point. The in-memory backend does no I/O, so
/// there is nothing to await while holding the guard. The file backend in Task 5 has to be
/// more careful: it must build the event payload under the lock, drop the guard, then call
/// `spawn_blocking` for the write. A guard held across an `.await` makes the future `!Send`,
/// which will not compile behind `Arc<dyn ReviewStore>`.
///
/// [`FileReviewStore`]: crate::review::store_file::FileReviewStore
#[derive(Debug, Default)]
pub struct InMemoryReviewStore {
    items: Mutex<Vec<ReviewQueueItem>>,
}

impl InMemoryReviewStore {
    /// Create a new, empty store.
    pub fn new() -> Self {
        Self::default()
    }

    /// Wrap this store in an `Arc` so it satisfies `Arc<dyn ReviewStore>`.
    pub fn into_arc(self) -> Arc<dyn ReviewStore> {
        Arc::new(self)
    }

    /// Acquire the mutex, recovering from poison.
    ///
    /// A reviewer thread panicking mid-update leaves the items in the state they were
    /// at the panic boundary. Refusing to serve subsequent callers would discard every
    /// pending decision for what is usually a bug in unrelated code. Recovery mirrors the
    /// same decision taken in `facade.rs` and `audit/store.rs`.
    fn lock(&self) -> std::sync::MutexGuard<'_, Vec<ReviewQueueItem>> {
        self.items
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

#[async_trait]
impl ReviewStore for InMemoryReviewStore {
    async fn submit(&self, item: ReviewQueueItem) -> Result<ReviewQueueItem, ReviewError> {
        // Guard acquired, item pushed, guard dropped before returning. No .await while held.
        self.lock().push(item.clone());
        Ok(item)
    }

    async fn assign(&self, id: &str, reviewer: &str) -> Result<ReviewQueueItem, ReviewError> {
        // The entire check-and-mutate sequence runs under one guard acquisition. This is
        // what preserves the compare-and-swap property documented on the trait. Releasing
        // the guard between finding the item and updating it would open a window where two
        // concurrent callers both see `Pending` and both proceed — splitting this across two
        // operations is a correctness bug, not an optimisation.
        let mut items = self.lock();
        let item = items
            .iter_mut()
            .find(|i| i.id == id)
            .ok_or_else(|| ReviewError::NotFound(id.to_string()))?;

        if item.status != ReviewStatus::Pending {
            return Err(ReviewError::InvalidTransition {
                from: item.status.to_string(),
                to: ReviewStatus::InReview.to_string(),
            });
        }

        item.assigned_reviewer = Some(reviewer.to_string());
        item.status = ReviewStatus::InReview;

        Ok(item.clone())
    }

    async fn decide(
        &self,
        id: &str,
        decision: ReviewDecision,
        reviewer: &str,
        comment: &str,
    ) -> Result<ReviewQueueItem, ReviewError> {
        // Same single-acquisition discipline as `assign`. The check on `decision.is_none()`
        // and the write of `decision` must happen inside one guard to prevent two callers
        // both observing "no decision yet" and both recording one.
        let mut items = self.lock();
        let item = items
            .iter_mut()
            .find(|i| i.id == id)
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
        item.decided_at = Some(chrono::Utc::now().to_rfc3339());
        item.comment = Some(comment.to_string());

        Ok(item.clone())
    }

    async fn list(
        &self,
        filter: Option<ReviewStatus>,
    ) -> Result<Vec<ReviewQueueItem>, ReviewError> {
        let items = self.lock();
        let result = match filter {
            Some(status) => items
                .iter()
                .filter(|i| i.status == status)
                .cloned()
                .collect(),
            None => items.clone(),
        };
        Ok(result)
    }

    async fn get(&self, id: &str) -> Result<Option<ReviewQueueItem>, ReviewError> {
        Ok(self.lock().iter().find(|i| i.id == id).cloned())
    }

    async fn stats(&self) -> Result<QueueStats, ReviewError> {
        let items = self.lock();
        let count = |status: ReviewStatus| items.iter().filter(|i| i.status == status).count();
        Ok(QueueStats {
            total: items.len(),
            pending: count(ReviewStatus::Pending),
            in_review: count(ReviewStatus::InReview),
            approved: count(ReviewStatus::Approved),
            rejected: count(ReviewStatus::Rejected),
            modified: count(ReviewStatus::Modified),
        })
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// Confirm that `Arc<dyn ReviewStore>` is constructible.
    ///
    /// If the trait is not object-safe — because a method is generic, returns `Self`, or
    /// uses an unsized associated type — this test fails to compile. It is deliberately
    /// placed before any backend-level tests so the property is proven at the trait level.
    #[tokio::test]
    async fn should_construct_arc_dyn_review_store() {
        let store = InMemoryReviewStore::new();
        let _: Arc<dyn ReviewStore> = Arc::new(store);
        // If this compiles, the trait is object-safe.
    }
}
