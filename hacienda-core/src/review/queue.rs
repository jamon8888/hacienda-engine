//! In-memory review queue for human-in-the-loop PII decisions.

use chrono::{Duration, Utc};
use std::sync::Mutex;
use uuid::Uuid;

use crate::review::error::ReviewError;
use crate::review::types::{
    Priority, QueueStats, ReviewConfig, ReviewDecision, ReviewQueueItem, ReviewRequest,
    ReviewStatus,
};

/// Thread-safe queue of low-confidence detections awaiting human review.
pub struct ReviewQueue {
    items: Mutex<Vec<ReviewQueueItem>>,
    config: ReviewConfig,
}

impl ReviewQueue {
    pub fn new(config: ReviewConfig) -> Self {
        Self {
            items: Mutex::new(Vec::new()),
            config,
        }
    }

    /// Whether a detection at `confidence` is uncertain enough to need review.
    pub fn needs_review(&self, confidence: f32) -> bool {
        confidence < self.config.confidence_threshold
    }

    /// Add a detection to the queue and return the item created for it.
    pub fn submit(&self, request: ReviewRequest) -> ReviewQueueItem {
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
        };
        self.lock().push(item.clone());
        item
    }

    /// Record a reviewer's decision, moving the item to its terminal status.
    ///
    /// # Errors
    ///
    /// - [`ReviewError::NotFound`] if no item has that id.
    /// - [`ReviewError::AlreadyDecided`] if the item already carries a decision.
    pub fn decide(
        &self,
        id: &str,
        decision: ReviewDecision,
        reviewer: &str,
        comment: &str,
    ) -> Result<ReviewQueueItem, ReviewError> {
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
        item.decided_at = Some(Utc::now().to_rfc3339());
        item.comment = Some(comment.to_string());

        Ok(item.clone())
    }

    /// Assign a reviewer, moving a pending item to [`ReviewStatus::InReview`].
    ///
    /// # Errors
    ///
    /// - [`ReviewError::NotFound`] if no item has that id.
    /// - [`ReviewError::InvalidTransition`] if the item is not pending.
    pub fn assign(&self, id: &str, reviewer: &str) -> Result<ReviewQueueItem, ReviewError> {
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

    /// List items, optionally restricted to a single status.
    pub fn list(&self, filter: Option<ReviewStatus>) -> Vec<ReviewQueueItem> {
        let items = self.lock();
        match filter {
            Some(status) => items
                .iter()
                .filter(|i| i.status == status)
                .cloned()
                .collect(),
            None => items.clone(),
        }
    }

    pub fn get(&self, id: &str) -> Option<ReviewQueueItem> {
        self.lock().iter().find(|i| i.id == id).cloned()
    }

    pub fn stats(&self) -> QueueStats {
        let items = self.lock();
        let count = |status: ReviewStatus| items.iter().filter(|i| i.status == status).count();
        QueueStats {
            total: items.len(),
            pending: count(ReviewStatus::Pending),
            in_review: count(ReviewStatus::InReview),
            approved: count(ReviewStatus::Approved),
            rejected: count(ReviewStatus::Rejected),
            modified: count(ReviewStatus::Modified),
        }
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

    /// A poisoned queue mutex means a reviewer thread panicked mid-update; the
    /// remaining items are still valid, so recover rather than propagating the panic.
    fn lock(&self) -> std::sync::MutexGuard<'_, Vec<ReviewQueueItem>> {
        self.items.lock().unwrap_or_else(|e| e.into_inner())
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

    #[test]
    fn should_queue_a_submitted_item_as_pending() {
        let queue = ReviewQueue::default();
        let item = queue.submit(request(0.4));
        assert_eq!(item.status, ReviewStatus::Pending);
        assert_eq!(queue.stats().pending, 1);
    }

    #[test]
    fn should_assign_a_deadline_from_the_config() {
        let queue = ReviewQueue::new(ReviewConfig {
            deadline_hours: Some(4),
            ..Default::default()
        });
        assert!(queue.submit(request(0.4)).deadline.is_some());
    }

    #[test]
    fn should_leave_the_deadline_unset_when_none_is_configured() {
        let queue = ReviewQueue::new(ReviewConfig {
            deadline_hours: None,
            ..Default::default()
        });
        assert!(queue.submit(request(0.4)).deadline.is_none());
    }

    #[test]
    fn should_flag_detections_below_the_confidence_threshold_for_review() {
        let queue = ReviewQueue::new(ReviewConfig {
            confidence_threshold: 0.6,
            ..Default::default()
        });
        assert!(queue.needs_review(0.59));
        assert!(!queue.needs_review(0.6));
    }

    #[test]
    fn should_derive_priority_from_confidence() {
        assert_eq!(
            ReviewQueue::priority_from_confidence(0.1),
            Priority::Critical
        );
        assert_eq!(ReviewQueue::priority_from_confidence(0.4), Priority::High);
        assert_eq!(ReviewQueue::priority_from_confidence(0.6), Priority::Normal);
        assert_eq!(ReviewQueue::priority_from_confidence(0.9), Priority::Low);
    }

    #[test]
    fn should_move_an_approved_item_out_of_pending() {
        let queue = ReviewQueue::default();
        let item = queue.submit(request(0.4));
        let decided = queue
            .decide(&item.id, ReviewDecision::Approve, "dpo", "looks right")
            .unwrap();

        assert_eq!(decided.status, ReviewStatus::Approved);
        assert_eq!(decided.decided_by.as_deref(), Some("dpo"));
        let stats = queue.stats();
        assert_eq!(stats.pending, 0);
        assert_eq!(stats.approved, 1);
    }

    #[test]
    fn should_reject_a_second_decision_on_the_same_item() {
        let queue = ReviewQueue::default();
        let item = queue.submit(request(0.4));
        queue
            .decide(&item.id, ReviewDecision::Approve, "dpo", "")
            .unwrap();
        assert!(matches!(
            queue.decide(&item.id, ReviewDecision::Reject, "dpo", ""),
            Err(ReviewError::AlreadyDecided(_))
        ));
    }

    #[test]
    fn should_report_not_found_for_an_unknown_id() {
        let queue = ReviewQueue::default();
        assert!(matches!(
            queue.decide("nope", ReviewDecision::Approve, "dpo", ""),
            Err(ReviewError::NotFound(_))
        ));
        assert!(queue.get("nope").is_none());
    }

    #[test]
    fn should_move_an_assigned_item_into_review() {
        let queue = ReviewQueue::default();
        let item = queue.submit(request(0.4));
        let assigned = queue.assign(&item.id, "bob").unwrap();
        assert_eq!(assigned.status, ReviewStatus::InReview);
        assert_eq!(assigned.assigned_reviewer.as_deref(), Some("bob"));
    }

    #[test]
    fn should_refuse_to_assign_an_item_that_is_no_longer_pending() {
        let queue = ReviewQueue::default();
        let item = queue.submit(request(0.4));
        queue.assign(&item.id, "bob").unwrap();
        assert!(matches!(
            queue.assign(&item.id, "carol"),
            Err(ReviewError::InvalidTransition { .. })
        ));
    }

    #[test]
    fn should_list_only_items_matching_the_status_filter() {
        let queue = ReviewQueue::default();
        let a = queue.submit(request(0.4));
        queue.submit(request(0.4));
        queue.assign(&a.id, "bob").unwrap();

        assert_eq!(queue.list(None).len(), 2);
        assert_eq!(queue.list(Some(ReviewStatus::Pending)).len(), 1);
        assert_eq!(queue.list(Some(ReviewStatus::InReview)).len(), 1);
    }

    #[test]
    fn should_accept_concurrent_submissions_from_many_threads() {
        let queue = std::sync::Arc::new(ReviewQueue::default());
        let handles: Vec<_> = (0..8)
            .map(|_| {
                let queue = std::sync::Arc::clone(&queue);
                std::thread::spawn(move || {
                    for _ in 0..25 {
                        queue.submit(request(0.4));
                    }
                })
            })
            .collect();
        for h in handles {
            h.join().unwrap();
        }
        assert_eq!(queue.stats().total, 200);
    }
}
