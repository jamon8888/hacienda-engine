//! Core types for the human review queue.

use serde::{Deserialize, Serialize};

/// A single item awaiting (or having received) human review.
#[derive(Debug, Clone, Serialize, Deserialize)]
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewStatus {
    Pending,
    InReview,
    Approved,
    Rejected,
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

/// Urgency of a review item, derived from detection confidence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Priority {
    Low,
    Normal,
    #[default]
    High,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ReviewDecision {
    #[default]
    Approve,
    Reject,
    Modify,
}

/// Submission payload for a new review item.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReviewRequest {
    pub text_snippet: String,
    pub category: String,
    pub start: u32,
    pub end: u32,
    pub confidence: f32,
    pub source: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct QueueStats {
    pub total: usize,
    pub pending: usize,
    pub in_review: usize,
    pub approved: usize,
    pub rejected: usize,
    pub modified: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
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
