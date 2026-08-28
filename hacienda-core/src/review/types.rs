//! Core types for the human review queue.

use serde::{Deserialize, Serialize};

/// A single item awaiting (or having received) human review.
///
/// # Tenant scoping (S1b)
///
/// `tenant_id` travels with the item rather than being a repeated parameter on every
/// [`crate::review::store::ReviewStore`] method — see that trait's doc and
/// `tenancy.rs`'s module doc (decision D-S1-1) for why a store method still takes
/// `&TenantId` explicitly wherever it cannot resolve the tenant from an item it already
/// has in hand (`list`/`get`/`stats`, and the pre-lookup half of `assign`/`decide`).
#[derive(Debug, Clone, Serialize, Deserialize)]
/// ReviewQueueItem struct
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
    /// The tenant this item belongs to (S1). Set by
    /// [`ReviewQueue::submit`](crate::review::ReviewQueue::submit) — always `"default"`
    /// until a caller uses
    /// [`ReviewQueue::submit_for_tenant`](crate::review::ReviewQueue::submit_for_tenant).
    ///
    /// Defaulted on deserialize: a record written by `FileReviewStore` before S1b
    /// shipped this field has no `tenant_id` key in its JSONL line at all, and every
    /// such record was, in effect, created for the single pre-tenancy deployment this
    /// default names explicitly. Without it, `FileReviewStore::open`'s replay would
    /// fail outright on any log written before this change (`review/store_file.rs`
    /// reads exactly this struct back via `serde_json`).
    #[serde(default = "default_tenant_id")]
    pub tenant_id: String,
}

fn default_tenant_id() -> String {
    crate::tenancy::DEFAULT_TENANT.to_owned()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
/// ReviewStatus enum
pub enum ReviewStatus {
    /// Pending variant
    Pending,
    /// InReview variant
    InReview,
    /// Approved variant
    Approved,
    /// Rejected variant
    Rejected,
    /// Modified variant
    Modified,
}

impl std::fmt::Display for ReviewStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            ReviewStatus::Pending => "pending",
            ReviewStatus::InReview => "in_review",
            ReviewStatus::Approved => "approved",
            ReviewStatus::Rejected => "rejected",
            ReviewStatus::Modified => "modified",
        };
        f.write_str(s)
    }
}

impl std::str::FromStr for ReviewStatus {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "pending" => Ok(ReviewStatus::Pending),
            "in_review" => Ok(ReviewStatus::InReview),
            "approved" => Ok(ReviewStatus::Approved),
            "rejected" => Ok(ReviewStatus::Rejected),
            "modified" => Ok(ReviewStatus::Modified),
            other => Err(format!("unknown review status '{other}'")),
        }
    }
}

/// Urgency of a review item, derived from detection confidence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
/// Priority enum
pub enum Priority {
    /// Low variant
    Low,
    /// Normal variant
    Normal,
    #[default]
    /// High variant
    High,
    /// Critical variant
    Critical,
}

impl std::fmt::Display for Priority {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            Priority::Low => "low",
            Priority::Normal => "normal",
            Priority::High => "high",
            Priority::Critical => "critical",
        };
        f.write_str(s)
    }
}

impl std::str::FromStr for Priority {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "low" => Ok(Priority::Low),
            "normal" => Ok(Priority::Normal),
            "high" => Ok(Priority::High),
            "critical" => Ok(Priority::Critical),
            other => Err(format!("unknown priority '{other}'")),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
/// ReviewDecision enum
pub enum ReviewDecision {
    #[default]
    /// Approve variant
    Approve,
    /// Reject variant
    Reject,
    /// Modify variant
    Modify,
}

impl std::fmt::Display for ReviewDecision {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            ReviewDecision::Approve => "approve",
            ReviewDecision::Reject => "reject",
            ReviewDecision::Modify => "modify",
        };
        f.write_str(s)
    }
}

impl std::str::FromStr for ReviewDecision {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "approve" => Ok(ReviewDecision::Approve),
            "reject" => Ok(ReviewDecision::Reject),
            "modify" => Ok(ReviewDecision::Modify),
            other => Err(format!("unknown review decision '{other}'")),
        }
    }
}

/// Submission payload for a new review item.
#[derive(Debug, Clone, Serialize, Deserialize)]
/// ReviewRequest struct
pub struct ReviewRequest {
    pub text_snippet: String,
    pub category: String,
    pub start: u32,
    pub end: u32,
    pub confidence: f32,
    pub source: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
/// QueueStats struct
pub struct QueueStats {
    pub total: usize,
    pub pending: usize,
    pub in_review: usize,
    pub approved: usize,
    pub rejected: usize,
    pub modified: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
/// ReviewConfig struct
pub struct ReviewConfig {
    /// Detections at or above this confidence never enter the queue.
    pub confidence_threshold: f32,
    pub deadline_hours: Option<u64>,
}

impl Default for ReviewConfig {
    fn default() -> Self {
        Self {
            confidence_threshold: 0.5,
            deadline_hours: Some(24),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A JSONL line written by `FileReviewStore` before S1b shipped the `tenant_id`
    /// field has no `tenant_id` key at all. Without `#[serde(default = "default_tenant_id")]`
    /// this fails to deserialize, and `FileReviewStore::open`'s replay of an
    /// existing log from before this change would error on its very first line.
    #[test]
    fn legacy_record_with_no_tenant_key_deserializes_to_the_default_tenant() {
        let legacy_json = r#"{
            "id": "item-1",
            "text_snippet": "jane@example.com",
            "category": "email",
            "start": 0,
            "end": 16,
            "confidence": 0.4,
            "source": "regex",
            "status": "pending",
            "priority": "normal",
            "assigned_reviewer": null,
            "created_at": "2026-01-01T00:00:00Z",
            "deadline": null,
            "decision": null,
            "decided_by": null,
            "decided_at": null,
            "comment": null
        }"#;

        let item: ReviewQueueItem = serde_json::from_str(legacy_json).unwrap();
        assert_eq!(item.tenant_id, default_tenant_id());
    }

    /// A record that does carry a `tenant_id` key still deserializes to that tenant,
    /// not the default — the `#[serde(default = ...)]` must only kick in when the key
    /// is absent, never override an explicit value.
    #[test]
    fn record_with_an_explicit_tenant_key_keeps_it() {
        let json = r#"{
            "id": "item-1",
            "tenant_id": "acme",
            "text_snippet": "jane@example.com",
            "category": "email",
            "start": 0,
            "end": 16,
            "confidence": 0.4,
            "source": "regex",
            "status": "pending",
            "priority": "normal",
            "assigned_reviewer": null,
            "created_at": "2026-01-01T00:00:00Z",
            "deadline": null,
            "decision": null,
            "decided_by": null,
            "decided_at": null,
            "comment": null
        }"#;

        let item: ReviewQueueItem = serde_json::from_str(json).unwrap();
        assert_eq!(item.tenant_id, "acme");
    }
}
