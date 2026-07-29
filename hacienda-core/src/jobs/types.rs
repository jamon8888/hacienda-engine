//! Core types for async job tracking.
//!
//! These types exist as a seam for Phase 4's HTTP endpoints (`POST /v1/documents/async`
//! and `GET /v1/jobs/{id}`). Nothing in the repo produces job state today; this module
//! establishes the vocabulary before the endpoints build on it so they are not
//! retrofitting a design onto a finished API surface.

use serde::{Deserialize, Serialize};

/// Lifecycle state of an async document-processing job.
///
/// The only legal progressions are `Queued → Running → Succeeded` and
/// `Queued → Running → Failed`. The `transition` method on [`super::store::JobStore`]
/// enforces this atomically: two workers racing to claim a `Queued` job will both call
/// `transition(id, Queued, Running)` and exactly one will succeed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobStatus {
    /// Job has been accepted and is waiting for a worker to claim it.
    Queued,
    /// A worker has claimed the job and is processing it.
    Running,
    /// Processing completed successfully; `result_json` is populated.
    Succeeded,
    /// Processing failed; `error` carries the reason.
    Failed,
}

impl std::fmt::Display for JobStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            JobStatus::Queued => "queued",
            JobStatus::Running => "running",
            JobStatus::Succeeded => "succeeded",
            JobStatus::Failed => "failed",
        };
        f.write_str(s)
    }
}

/// An async document-processing job.
///
/// Created by [`super::store::JobStore::create`] and updated atomically via
/// `transition`, `finish`, and `fail`. Callers that need to stream progress or
/// implement retries should poll `get` — no push notification mechanism is defined
/// here, because no consumer exists yet.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Job {
    /// Stable identifier — UUID v4, assigned on creation.
    pub id: String,
    /// Current lifecycle state.
    pub status: JobStatus,
    /// RFC 3339 timestamp when the job was created.
    pub created_at: String,
    /// RFC 3339 timestamp of the most recent status change.
    pub updated_at: String,
    /// Serialised result payload from the pipeline, populated on `Succeeded`.
    ///
    /// Stored as opaque JSON so the store layer does not depend on the pipeline's
    /// result type — a store that names `HaciendaResult` cannot be reused by any other
    /// pipeline, and `Arc<dyn JobStore>` cannot be generic. See plan D5.
    pub result_json: Option<String>,
    /// Human-readable failure reason, populated on `Failed`.
    pub error: Option<String>,
}
