mod config;
mod decision;
mod item;
mod queue;

pub use config::ReviewConfig;
pub use decision::{Priority, ReviewDecision, ReviewStatus};
pub use item::{ReviewQueueItem, ReviewRequest};
pub use queue::ReviewQueue;

use pii_review::{
    Priority as PiiPriority, ReviewConfig as PiiReviewConfig, ReviewDecision as PiiReviewDecision,
    ReviewQueue as PiiReviewQueue, ReviewQueueItem as PiiReviewQueueItem,
    ReviewRequest as PiiReviewRequest, ReviewStatus as PiiReviewStatus,
};

pub struct ReviewQueueWrapper {
    inner: PiiReviewQueue,
}

impl ReviewQueueWrapper {
    pub fn new(config: ReviewConfig) -> Self {
        let pii_config = PiiReviewConfig::default();
        Self {
            inner: PiiReviewQueue::new(),
        }
    }

    pub fn submit(&self, request: ReviewRequest) -> ReviewQueueItem {
        let pii_request = PiiReviewRequest {
            text_snippet: request.text_snippet,
            category: request.category,
            start: request.start,
            end: request.end,
            confidence: request.confidence,
            source: request.source,
        };
        let item = self.inner.submit(pii_request);
        item.into()
    }

    pub fn decide(
        &self,
        id: &str,
        decision: ReviewDecision,
        reviewer: &str,
        comment: &str,
    ) -> Result<ReviewQueueItem, String> {
        let pii_decision = match decision {
            ReviewDecision::Approve => PiiReviewDecision::Approve,
            ReviewDecision::Reject => PiiReviewDecision::Reject,
            ReviewDecision::Modify => PiiReviewDecision::Modify,
        };
        self.inner
            .decide(id, pii_decision, reviewer, comment)
            .map(|i| i.into())
            .map_err(|e| e.to_string())
    }

    pub fn assign(&self, id: &str, reviewer: &str) -> Result<ReviewQueueItem, String> {
        self.inner
            .assign(id, reviewer)
            .map(|i| i.into())
            .map_err(|e| e.to_string())
    }

    pub fn list(&self, filter: Option<ReviewStatus>) -> Vec<ReviewQueueItem> {
        let pii_filter = filter.map(|s| match s {
            ReviewStatus::Pending => pii_review::ReviewStatus::Pending,
            ReviewStatus::InReview => pii_review::ReviewStatus::InReview,
            ReviewStatus::Approved => pii_review::ReviewStatus::Approved,
            ReviewStatus::Rejected => pii_review::ReviewStatus::Rejected,
            ReviewStatus::Modified => pii_review::ReviewStatus::Modified,
        });
        self.inner
            .list(pii_filter)
            .into_iter()
            .map(|i| i.into())
            .collect()
    }

    pub fn get(&self, id: &str) -> Option<ReviewQueueItem> {
        self.inner.get(id).map(|i| i.into())
    }

    pub fn stats(&self) -> QueueStats {
        let s = self.inner.stats();
        QueueStats {
            total: s.total,
            pending: s.pending,
            in_review: s.in_review,
            approved: s.approved,
            rejected: s.rejected,
            modified: s.modified,
        }
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ReviewQueueItem {
    pub id: String,
    pub text_snippet: String,
    pub category: String,
    pub start: u32,
    pub end: u32,
    pub confidence: f32,
    pub source: String,
    pub status: ReviewStatus,
    pub priority: Priority,
    pub assigned_reviewer: Option<String>,
    pub created_at: String,
    pub deadline: Option<String>,
    pub decision: Option<ReviewDecision>,
    pub decided_by: Option<String>,
    pub decided_at: Option<String>,
    pub comment: Option<String>,
}

impl From<pii_review::ReviewQueueItem> for ReviewQueueItem {
    fn from(i: pii_review::ReviewQueueItem) -> Self {
        Self {
            id: i.id,
            text_snippet: i.text_snippet,
            category: i.category,
            start: i.start,
            end: i.end,
            confidence: i.confidence,
            source: i.source,
            status: i.status.into(),
            priority: i.priority.into(),
            assigned_reviewer: i.assigned_reviewer,
            created_at: i.created_at,
            deadline: i.deadline,
            decision: i.decision.map(|d| d.into()),
            decided_by: i.decided_by,
            decided_at: i.decided_at,
            comment: i.comment,
        }
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ReviewRequest {
    pub text_snippet: String,
    pub category: String,
    pub start: u32,
    pub end: u32,
    pub confidence: f32,
    pub source: String,
}

impl From<ReviewRequest> for pii_review::ReviewRequest {
    fn from(r: ReviewRequest) -> Self {
        Self {
            text_snippet: r.text_snippet,
            category: r.category,
            start: r.start,
            end: r.end,
            confidence: r.confidence,
            source: r.source,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewStatus {
    Pending,
    InReview,
    Approved,
    Rejected,
    Modified,
}

impl From<pii_review::ReviewStatus> for ReviewStatus {
    fn from(s: pii_review::ReviewStatus) -> Self {
        match s {
            pii_review::ReviewStatus::Pending => Self::Pending,
            pii_review::ReviewStatus::InReview => Self::InReview,
            pii_review::ReviewStatus::Approved => Self::Approved,
            pii_review::ReviewStatus::Rejected => Self::Rejected,
            pii_review::ReviewStatus::Modified => Self::Modified,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewDecision {
    Approve,
    Reject,
    Modify,
}

impl From<pii_review::ReviewDecision> for ReviewDecision {
    fn from(d: pii_review::ReviewDecision) -> Self {
        match d {
            pii_review::ReviewDecision::Approve => Self::Approve,
            pii_review::ReviewDecision::Reject => Self::Reject,
            pii_review::ReviewDecision::Modify => Self::Modify,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Priority {
    Low,
    Normal,
    High,
    Critical,
}

impl Default for Priority {
    fn default() -> Self {
        Self::High
    }
}

impl From<pii_review::Priority> for Priority {
    fn from(p: pii_review::Priority) -> Self {
        match p {
            pii_review::Priority::Low => Self::Low,
            pii_review::Priority::Normal => Self::Normal,
            pii_review::Priority::High => Self::High,
            pii_review::Priority::Critical => Self::Critical,
        }
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct QueueStats {
    pub total: usize,
    pub pending: usize,
    pub in_review: usize,
    pub approved: usize,
    pub rejected: usize,
    pub modified: usize,
}
