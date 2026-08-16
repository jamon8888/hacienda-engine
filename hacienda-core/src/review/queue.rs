//! Human review queue — policy wrapper around a [`ReviewStore`].
//!
//! [`ReviewQueue`] is responsible for deciding *whether* a detection needs review and
//! *which priority and deadline* to assign it. It is not responsible for storing the result:
//! that is the job of the [`ReviewStore`] it holds.
//!
//! Separating policy from persistence means the same queue logic works unchanged in front of
//! an in-memory store (the default, appropriate for CLI use), a file store (Task 5), or a
//! Postgres store (Phase 6).
//!
//! [`ReviewStore`]: crate::review::store::ReviewStore

use chrono::{Duration, Utc};
use std::sync::Arc;
use uuid::Uuid;

use crate::review::error::ReviewError;
use crate::review::store::{InMemoryReviewStore, ReviewStore};
use crate::review::types::{
    Priority, QueueStats, ReviewConfig, ReviewDecision, ReviewQueueItem, ReviewRequest,
    ReviewStatus,
};
use crate::tenancy::TenantId;

/// Policy wrapper around a [`ReviewStore`].
///
/// Holds the configuration that governs when an item enters the queue and what priority and
/// deadline it gets. The actual persistence of items is delegated to the store.
///
/// # Constructors
///
/// - [`ReviewQueue::new`] — default in-memory store, suitable for tests and CLI use.
/// - [`ReviewQueue::with_store`] — supply an explicit store (file, Postgres, or a test double).
pub struct ReviewQueue {
    store: Arc<dyn ReviewStore>,
    config: ReviewConfig,
}

impl ReviewQueue {
    /// Create a queue backed by a fresh in-memory store.
    ///
    /// Review state will be lost when this value is dropped. For processes that must survive
    /// a restart use [`with_store`](Self::with_store) and pass a durable backend.
    pub fn new(config: ReviewConfig) -> Self {
        Self {
            store: Arc::new(InMemoryReviewStore::new()),
            config,
        }
    }

    /// Create a queue backed by the given store.
    ///
    /// Use this when the caller needs to swap in a file or database backend, or to share a
    /// store with other parts of the system (e.g., the facade in Task 7).
    pub fn with_store(config: ReviewConfig, store: Arc<dyn ReviewStore>) -> Self {
        Self { store, config }
    }

    /// Whether a detection at `confidence` is uncertain enough to need review.
    pub fn needs_review(&self, confidence: f32) -> bool {
        confidence < self.config.confidence_threshold
    }

    /// Add a detection to the queue and return the item created for it.
    ///
    /// Policy lives here: `id`, `priority`, `deadline`, and `created_at` are all set in
    /// this method before the item is handed to the store. The store records exactly what it
    /// receives and returns it unchanged.
    ///
    /// # Errors
    ///
    /// Whatever the backing store returns. This was infallible while the queue owned its
    /// own `Vec`, and became fallible when the store seam was introduced — a durable
    /// backend can fail to write. The error is propagated rather than absorbed: an item
    /// that never reached the store is an item no reviewer will ever see, and reporting
    /// that as success would hide a compliance failure behind a clean return value.
    pub async fn submit(
        &self,
        tenant: &TenantId,
        request: ReviewRequest,
    ) -> Result<ReviewQueueItem, ReviewError> {
        let deadline = self
            .config
            .deadline_hours
            .and_then(|h| i64::try_from(h).ok())
            .map(|h| (Utc::now() + Duration::hours(h)).to_rfc3339());

        let item = ReviewQueueItem {
            id: Uuid::new_v4().to_string(),
            priority: Self::priority_from_confidence(request.confidence),
            text_snippet: request.text_snippet,
            category: request.category,
            start: request.start,
            end: request.end,
            confidence: request.confidence,
            source: request.source,
            status: ReviewStatus::Pending,
            assigned_reviewer: None,
            created_at: Utc::now().to_rfc3339(),
            deadline,
            decision: None,
            decided_by: None,
            decided_at: None,
            comment: None,
            tenant_id: tenant.to_string(),
        };

        self.store.submit(tenant, item).await
    }

    /// Record a reviewer's decision, scoped to `tenant` (S1).
    ///
    /// # Errors
    ///
    /// - [`ReviewError::NotFound`] if no item has that id in `tenant` — including an
    ///   id that exists but belongs to a different tenant, see [`ReviewStore::assign`]'s doc.
    /// - [`ReviewError::AlreadyDecided`] if the item already carries a decision.
    pub async fn decide(
        &self,
        tenant: &TenantId,
        id: &str,
        decision: ReviewDecision,
        reviewer: &str,
        comment: &str,
    ) -> Result<ReviewQueueItem, ReviewError> {
        self.store
            .decide(tenant, id, decision, reviewer, comment)
            .await
    }

    /// Assign a reviewer, moving a pending item to [`ReviewStatus::InReview`], scoped to
    /// `tenant` (S1).
    ///
    /// # Errors
    ///
    /// - [`ReviewError::NotFound`] if no item has that id (including an id belonging to
    ///   a different tenant — see [`ReviewStore::assign`]'s doc).
    /// - [`ReviewError::InvalidTransition`] if the item is not pending.
    ///
    /// [`ReviewStore::assign`]: crate::review::store::ReviewStore::assign
    pub async fn assign(
        &self,
        tenant: &TenantId,
        id: &str,
        reviewer: &str,
    ) -> Result<ReviewQueueItem, ReviewError> {
        self.store.assign(tenant, id, reviewer).await
    }

    /// List `tenant`'s items, optionally restricted to a single status (S1).
    ///
    /// # Errors
    ///
    /// Whatever the backing store returns.
    ///
    /// This deliberately does **not** collapse a store failure into an empty list. An
    /// empty list is a meaningful answer — "nothing awaits review" — and a backend that
    /// cannot be read must not be able to impersonate it. A reviewer shown an empty queue
    /// by a broken store would reasonably conclude there was no work outstanding.
    pub async fn list(
        &self,
        tenant: &TenantId,
        filter: Option<ReviewStatus>,
    ) -> Result<Vec<ReviewQueueItem>, ReviewError> {
        self.store.list(tenant, filter).await
    }

    /// Retrieve a single item by id, scoped to `tenant` (S1).
    ///
    /// # Errors
    ///
    /// Whatever the backing store returns. Note the nesting: `Ok(None)` means the store
    /// was read and holds no such item (including an id belonging to a different
    /// tenant — see [`ReviewStore::get`]'s doc), while `Err(_)` means it could not be
    /// read at all. Flattening the two would let an unreadable store answer "no such
    /// item".
    ///
    /// [`ReviewStore::get`]: crate::review::store::ReviewStore::get
    pub async fn get(
        &self,
        tenant: &TenantId,
        id: &str,
    ) -> Result<Option<ReviewQueueItem>, ReviewError> {
        self.store.get(tenant, id).await
    }

    /// Return counts of `tenant`'s items in each status.
    ///
    /// # Errors
    ///
    /// Whatever the backing store returns. As with [`list`](Self::list), a failure must
    /// not present as all-zero counts — that is indistinguishable from an empty queue.
    pub async fn stats(&self, tenant: &TenantId) -> Result<QueueStats, ReviewError> {
        self.store.stats(tenant).await
    }

    /// Release the backing store's resources. Idempotent.
    ///
    /// # Errors
    ///
    /// Whatever [`ReviewStore::close`] returns.
    pub async fn close(&self) -> Result<(), ReviewError> {
        self.store.close().await
    }

    /// Map a detection confidence onto a review priority.
    ///
    /// Below 0.3 is Critical, below 0.5 High, below 0.7 Normal, otherwise Low.
    pub fn priority_from_confidence(confidence: f32) -> Priority {
        if confidence < 0.3 {
            Priority::Critical
        } else if confidence < 0.5 {
            Priority::High
        } else if confidence < 0.7 {
            Priority::Normal
        } else {
            Priority::Low
        }
    }
}

impl Default for ReviewQueue {
    fn default() -> Self {
        Self::new(ReviewConfig::default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The tenant used by every test that doesn't itself care about tenant scoping.
    fn t() -> TenantId {
        TenantId::new("tenant-a")
    }

    fn request(confidence: f32) -> ReviewRequest {
        ReviewRequest {
            text_snippet: "alice@example.com".into(),
            category: "Email".into(),
            start: 0,
            end: 17,
            confidence,
            source: "model".into(),
        }
    }

    #[tokio::test]
    async fn should_queue_a_submitted_item_as_pending() {
        let queue = ReviewQueue::default();
        let item = queue.submit(&t(), request(0.4)).await.expect("submit");
        assert_eq!(item.status, ReviewStatus::Pending);
        assert_eq!(queue.stats(&t()).await.expect("stats").pending, 1);
    }

    #[tokio::test]
    async fn should_assign_a_deadline_from_the_config() {
        let queue = ReviewQueue::new(ReviewConfig {
            deadline_hours: Some(4),
            ..Default::default()
        });
        assert!(queue
            .submit(&t(), request(0.4))
            .await
            .expect("submit")
            .deadline
            .is_some());
    }

    #[tokio::test]
    async fn should_leave_the_deadline_unset_when_none_is_configured() {
        let queue = ReviewQueue::new(ReviewConfig {
            deadline_hours: None,
            ..Default::default()
        });
        assert!(queue
            .submit(&t(), request(0.4))
            .await
            .expect("submit")
            .deadline
            .is_none());
    }

    #[tokio::test]
    async fn should_flag_detections_below_the_confidence_threshold_for_review() {
        let queue = ReviewQueue::new(ReviewConfig {
            confidence_threshold: 0.6,
            ..Default::default()
        });
        assert!(queue.needs_review(0.59));
        assert!(!queue.needs_review(0.6));
    }

    #[tokio::test]
    async fn should_derive_priority_from_confidence() {
        assert_eq!(
            ReviewQueue::priority_from_confidence(0.1),
            Priority::Critical
        );
        assert_eq!(ReviewQueue::priority_from_confidence(0.4), Priority::High);
        assert_eq!(ReviewQueue::priority_from_confidence(0.6), Priority::Normal);
        assert_eq!(ReviewQueue::priority_from_confidence(0.9), Priority::Low);
    }

    #[tokio::test]
    async fn should_move_an_approved_item_out_of_pending() {
        let queue = ReviewQueue::default();
        let item = queue.submit(&t(), request(0.4)).await.expect("submit");
        let decided = queue
            .decide(
                &t(),
                &item.id,
                ReviewDecision::Approve,
                "dpo",
                "looks right",
            )
            .await
            .unwrap();

        assert_eq!(decided.status, ReviewStatus::Approved);
        assert_eq!(decided.decided_by.as_deref(), Some("dpo"));
        let stats = queue.stats(&t()).await.expect("stats");
        assert_eq!(stats.pending, 0);
        assert_eq!(stats.approved, 1);
    }

    #[tokio::test]
    async fn should_reject_a_second_decision_on_the_same_item() {
        let queue = ReviewQueue::default();
        let item = queue.submit(&t(), request(0.4)).await.expect("submit");
        queue
            .decide(&t(), &item.id, ReviewDecision::Approve, "dpo", "")
            .await
            .unwrap();
        assert!(matches!(
            queue
                .decide(&t(), &item.id, ReviewDecision::Reject, "dpo", "")
                .await,
            Err(ReviewError::AlreadyDecided(_))
        ));
    }

    #[tokio::test]
    async fn should_report_not_found_for_an_unknown_id() {
        let queue = ReviewQueue::default();
        assert!(matches!(
            queue
                .decide(&t(), "nope", ReviewDecision::Approve, "dpo", "")
                .await,
            Err(ReviewError::NotFound(_))
        ));
        assert!(queue.get(&t(), "nope").await.expect("get").is_none());
    }

    #[tokio::test]
    async fn should_move_an_assigned_item_into_review() {
        let queue = ReviewQueue::default();
        let item = queue.submit(&t(), request(0.4)).await.expect("submit");
        let assigned = queue.assign(&t(), &item.id, "bob").await.unwrap();
        assert_eq!(assigned.status, ReviewStatus::InReview);
        assert_eq!(assigned.assigned_reviewer.as_deref(), Some("bob"));
    }

    #[tokio::test]
    async fn should_refuse_to_assign_an_item_that_is_no_longer_pending() {
        let queue = ReviewQueue::default();
        let item = queue.submit(&t(), request(0.4)).await.expect("submit");
        queue.assign(&t(), &item.id, "bob").await.unwrap();
        assert!(matches!(
            queue.assign(&t(), &item.id, "carol").await,
            Err(ReviewError::InvalidTransition { .. })
        ));
    }

    #[tokio::test]
    async fn should_list_only_items_matching_the_status_filter() {
        let queue = ReviewQueue::default();
        let a = queue.submit(&t(), request(0.4)).await.expect("submit");
        queue.submit(&t(), request(0.4)).await.expect("submit");
        queue.assign(&t(), &a.id, "bob").await.unwrap();

        assert_eq!(queue.list(&t(), None).await.expect("list").len(), 2);
        assert_eq!(
            queue
                .list(&t(), Some(ReviewStatus::Pending))
                .await
                .expect("list")
                .len(),
            1
        );
        assert_eq!(
            queue
                .list(&t(), Some(ReviewStatus::InReview))
                .await
                .expect("list")
                .len(),
            1
        );
    }

    /// Two concurrent reviewers both try to assign themselves to the same pending item.
    ///
    /// The CAS in `InMemoryReviewStore::assign` guarantees exactly one wins. This test is
    /// the regression net for the refactor from `Mutex<Vec>` in `ReviewQueue` to
    /// `Arc<dyn ReviewStore>`: if the refactor degrades `assign` into a read-then-write
    /// pair, both callers will observe `Pending` and both will succeed, breaking this
    /// assertion. Eight tasks amplify the race far enough to surface it reliably.
    #[tokio::test(flavor = "multi_thread", worker_threads = 8)]
    async fn should_let_exactly_one_of_two_concurrent_assignments_win() {
        use tokio::sync::Barrier;

        const TASKS: usize = 8;
        let queue = Arc::new(ReviewQueue::default());
        let item = queue.submit(&t(), request(0.4)).await.expect("submit");
        let id = item.id.clone();
        let barrier = Arc::new(Barrier::new(TASKS));

        let mut handles = Vec::with_capacity(TASKS);
        for task in 0..TASKS {
            let queue = Arc::clone(&queue);
            let barrier = Arc::clone(&barrier);
            let id = id.clone();
            handles.push(tokio::spawn(async move {
                barrier.wait().await;
                queue.assign(&t(), &id, &format!("reviewer-{task}")).await
            }));
        }

        let mut results = Vec::with_capacity(TASKS);
        for h in handles {
            results.push(h.await.expect("task must not panic"));
        }

        let ok_count = results.iter().filter(|r| r.is_ok()).count();
        let err_count = results
            .iter()
            .filter(|r| matches!(r, Err(ReviewError::InvalidTransition { .. })))
            .count();

        assert_eq!(ok_count, 1, "exactly one assignment should win");
        assert_eq!(
            err_count,
            TASKS - 1,
            "all other attempts must get InvalidTransition"
        );

        let winner = results.into_iter().find_map(|r| r.ok()).unwrap();
        let stored = queue
            .get(&t(), &id)
            .await
            .expect("get")
            .expect("the item exists");
        assert_eq!(stored.assigned_reviewer, winner.assigned_reviewer);
        assert_eq!(stored.status, ReviewStatus::InReview);
    }

    /// Two concurrent reviewers both try to decide on the same item.
    ///
    /// The CAS in `InMemoryReviewStore::decide` guarantees exactly one wins. This is the
    /// decision-path counterpart to `should_let_exactly_one_of_two_concurrent_assignments_win`.
    #[tokio::test(flavor = "multi_thread", worker_threads = 8)]
    async fn should_let_exactly_one_of_two_concurrent_decisions_win() {
        use tokio::sync::Barrier;

        const TASKS: usize = 8;
        let queue = Arc::new(ReviewQueue::default());
        let item = queue.submit(&t(), request(0.4)).await.expect("submit");
        let id = item.id.clone();
        let barrier = Arc::new(Barrier::new(TASKS));

        let mut handles = Vec::with_capacity(TASKS);
        for task in 0..TASKS {
            let queue = Arc::clone(&queue);
            let barrier = Arc::clone(&barrier);
            let id = id.clone();
            handles.push(tokio::spawn(async move {
                barrier.wait().await;
                queue
                    .decide(
                        &t(),
                        &id,
                        ReviewDecision::Approve,
                        &format!("reviewer-{task}"),
                        "ok",
                    )
                    .await
            }));
        }

        let mut results = Vec::with_capacity(TASKS);
        for h in handles {
            results.push(h.await.expect("task must not panic"));
        }

        let ok_count = results.iter().filter(|r| r.is_ok()).count();
        let err_count = results
            .iter()
            .filter(|r| matches!(r, Err(ReviewError::AlreadyDecided(_))))
            .count();

        assert_eq!(ok_count, 1, "exactly one decision should win");
        assert_eq!(
            err_count,
            TASKS - 1,
            "all other attempts must get AlreadyDecided"
        );

        let winner = results.into_iter().find_map(|r| r.ok()).unwrap();
        let stored = queue
            .get(&t(), &id)
            .await
            .expect("get")
            .expect("the item exists");
        assert_eq!(stored.decided_by, winner.decided_by);
        assert_eq!(stored.status, ReviewStatus::Approved);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn should_accept_concurrent_submissions_from_many_threads() {
        let queue = Arc::new(ReviewQueue::default());
        let mut handles = Vec::new();
        for _ in 0..8 {
            let queue = Arc::clone(&queue);
            handles.push(tokio::spawn(async move {
                for _ in 0..25 {
                    queue.submit(&t(), request(0.4)).await.expect("submit");
                }
            }));
        }
        for h in handles {
            h.await.unwrap();
        }
        assert_eq!(queue.stats(&t()).await.expect("stats").total, 200);
    }
}
