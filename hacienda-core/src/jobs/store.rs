//! [`JobStore`] trait and the in-memory backend.
//!
//! The trait was established in Phase 4 so that `POST /v1/documents/async` and
//! `GET /v1/jobs/{id}` had a concrete seam to build against rather than retrofitting one
//! onto a finished API surface. `/v1/documents/async` now exists as the real producer,
//! and [`crate::store::postgres::PostgresJobStore`] (Phase 9, gated behind the `postgres`
//! feature) is the real durable backend — this in-memory implementation remains for tests
//! and for deployments that accept losing queued jobs on restart (see the crate-level
//! `postgres` feature docs for the durability trade-off).

use super::{
    error::JobError,
    types::{Job, JobStatus},
};
use async_trait::async_trait;
use chrono::Utc;
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};
use uuid::Uuid;

/// Persistent store for async job state.
///
/// All mutating operations are atomic compare-and-swaps. The `transition` method is
/// the most important: it takes the *expected current status* (`from`) so that two
/// workers racing to claim the same `Queued` job will both call
/// `transition(id, Queued, Running)` and exactly one will win. A store that exposed
/// only `set_status` would need external coordination to achieve the same property.
///
/// Exercised by `/v1/jobs/{id}` and implemented by both [`InMemoryJobStore`] and
/// [`crate::store::postgres::PostgresJobStore`] (Phase 9) — the trait surface is the real,
/// stable seam between the async job API and its storage backend, not a placeholder.
#[async_trait]
pub trait JobStore: Send + Sync {
    /// Create a new job in the `Queued` state.
    ///
    /// `owner` is the principal id of the caller who submitted the job. Pass
    /// `None` for trusted in-process callers (CLI, desktop app). The transport
    /// layer uses this field to enforce per-tenant isolation — a principal who
    /// requests a job they do not own receives 404 rather than 403 so that the
    /// existence of another tenant's job is not disclosed (OWASP A01).
    ///
    /// The store records the value without enforcing it; isolation belongs at
    /// the transport layer where the HTTP status code is chosen.
    async fn create(&self, owner: Option<String>) -> Result<Job, JobError>;

    /// Retrieve a job by id, returning `None` if it does not exist.
    async fn get(&self, id: &str) -> Result<Option<Job>, JobError>;

    /// Atomically transition a job's status from `from` to `to`.
    ///
    /// Fails with [`JobError::StatusMismatch`] if the job's current status is not
    /// `from`. This compare-and-swap is what stops two workers both claiming the same
    /// queued job: only the one whose `from` matches the stored status wins.
    async fn transition(&self, id: &str, from: JobStatus, to: JobStatus) -> Result<Job, JobError>;

    /// Mark a job as `Succeeded` and record its serialised result payload.
    async fn finish(&self, id: &str, result_json: String) -> Result<Job, JobError>;

    /// Mark a job as `Failed` and record the reason.
    async fn fail(&self, id: &str, error: String) -> Result<Job, JobError>;

    /// List all jobs, optionally filtered by status. Ordered by creation time, oldest first.
    async fn list(&self, filter: Option<JobStatus>) -> Result<Vec<Job>, JobError>;
}

/// In-memory [`JobStore`] backed by a `Mutex<HashMap>`.
///
/// Appropriate for testing and for short-lived processes where durability is not
/// required. All operations complete without I/O, so the `Mutex` is never held across
/// an `.await`. Phase 4 will introduce a durable backend when a real producer exists.
#[derive(Debug, Default)]
pub struct InMemoryJobStore {
    jobs: Mutex<HashMap<String, Job>>,
}

impl InMemoryJobStore {
    /// Create a new, empty in-memory store.
    pub fn new() -> Self {
        Self::default()
    }

    /// Wrap this store in an `Arc` so it satisfies `Arc<dyn JobStore>`.
    pub fn into_arc(self) -> Arc<dyn JobStore> {
        Arc::new(self)
    }
}

fn now_rfc3339() -> String {
    Utc::now().to_rfc3339()
}

#[async_trait]
impl JobStore for InMemoryJobStore {
    async fn create(&self, owner: Option<String>) -> Result<Job, JobError> {
        let id = Uuid::new_v4().to_string();
        let now = now_rfc3339();
        let job = Job {
            id: id.clone(),
            status: JobStatus::Queued,
            created_at: now.clone(),
            updated_at: now,
            result_json: None,
            error: None,
            owner,
        };
        // Take the guard, insert, drop — no .await while held.
        let mut guard = self
            .jobs
            .lock()
            .map_err(|_| JobError::Internal("lock poisoned".into()))?;
        guard.insert(id, job.clone());
        Ok(job)
    }

    async fn get(&self, id: &str) -> Result<Option<Job>, JobError> {
        let guard = self
            .jobs
            .lock()
            .map_err(|_| JobError::Internal("lock poisoned".into()))?;
        Ok(guard.get(id).cloned())
    }

    async fn transition(&self, id: &str, from: JobStatus, to: JobStatus) -> Result<Job, JobError> {
        let mut guard = self
            .jobs
            .lock()
            .map_err(|_| JobError::Internal("lock poisoned".into()))?;
        let job = guard
            .get_mut(id)
            .ok_or_else(|| JobError::NotFound(id.to_owned()))?;
        if job.status != from {
            return Err(JobError::StatusMismatch {
                id: id.to_owned(),
                expected: from,
                actual: job.status,
            });
        }
        job.status = to;
        job.updated_at = now_rfc3339();
        Ok(job.clone())
    }

    async fn finish(&self, id: &str, result_json: String) -> Result<Job, JobError> {
        let mut guard = self
            .jobs
            .lock()
            .map_err(|_| JobError::Internal("lock poisoned".into()))?;
        let job = guard
            .get_mut(id)
            .ok_or_else(|| JobError::NotFound(id.to_owned()))?;
        job.status = JobStatus::Succeeded;
        job.result_json = Some(result_json);
        job.updated_at = now_rfc3339();
        Ok(job.clone())
    }

    async fn fail(&self, id: &str, error: String) -> Result<Job, JobError> {
        let mut guard = self
            .jobs
            .lock()
            .map_err(|_| JobError::Internal("lock poisoned".into()))?;
        let job = guard
            .get_mut(id)
            .ok_or_else(|| JobError::NotFound(id.to_owned()))?;
        job.status = JobStatus::Failed;
        job.error = Some(error);
        job.updated_at = now_rfc3339();
        Ok(job.clone())
    }

    async fn list(&self, filter: Option<JobStatus>) -> Result<Vec<Job>, JobError> {
        let guard = self
            .jobs
            .lock()
            .map_err(|_| JobError::Internal("lock poisoned".into()))?;
        let mut jobs: Vec<Job> = match filter {
            Some(status) => guard
                .values()
                .filter(|j| j.status == status)
                .cloned()
                .collect(),
            None => guard.values().cloned().collect(),
        };
        // Stable order: oldest created_at first, id as tie-break. The tie-break is not
        // decoration — RFC 3339 timestamps have finite resolution, so jobs created in the
        // same instant compare equal and would otherwise come back in `HashMap` iteration
        // order, which is randomised per process.
        jobs.sort_by(|a, b| {
            a.created_at
                .cmp(&b.created_at)
                .then_with(|| a.id.cmp(&b.id))
        });
        Ok(jobs)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn should_create_a_job_in_the_queued_state() {
        let store = InMemoryJobStore::new();
        let job = store.create(None).await.unwrap();
        assert_eq!(job.status, JobStatus::Queued);
        assert!(job.result_json.is_none());
        assert!(job.error.is_none());
        assert!(job.owner.is_none());
        // id is a valid UUID v4
        assert!(Uuid::parse_str(&job.id).is_ok());
    }

    #[tokio::test]
    async fn should_preserve_owner_on_create() {
        let store = InMemoryJobStore::new();
        let job = store.create(Some("alice".into())).await.unwrap();
        assert_eq!(job.owner.as_deref(), Some("alice"));

        // get round-trips the owner unchanged
        let fetched = store.get(&job.id).await.unwrap().unwrap();
        assert_eq!(fetched.owner.as_deref(), Some("alice"));
    }

    #[tokio::test]
    async fn should_accept_no_owner_for_trusted_callers() {
        let store = InMemoryJobStore::new();
        let job = store.create(None).await.unwrap();
        assert!(job.owner.is_none());

        let fetched = store.get(&job.id).await.unwrap().unwrap();
        assert!(fetched.owner.is_none());
    }

    #[tokio::test]
    async fn should_refuse_a_transition_from_an_unexpected_status() {
        let store = InMemoryJobStore::new();
        let job = store.create(None).await.unwrap();

        // Trying to transition from Running when it's actually Queued must fail.
        let err = store
            .transition(&job.id, JobStatus::Running, JobStatus::Succeeded)
            .await
            .unwrap_err();

        match err {
            JobError::StatusMismatch {
                id,
                expected,
                actual,
            } => {
                assert_eq!(id, job.id);
                assert_eq!(expected, JobStatus::Running);
                assert_eq!(actual, JobStatus::Queued);
            }
            other => panic!("expected StatusMismatch, got {other}"),
        }
    }

    /// Two tokio tasks race to claim the same queued job. The compare-and-swap in
    /// `transition` guarantees exactly one succeeds. A sequential test would not
    /// exercise the CAS property — both callers would succeed because the first
    /// changes the state the second reads.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn should_let_exactly_one_of_two_workers_claim_a_queued_job() {
        let store = Arc::new(InMemoryJobStore::new());
        let job = store.create(None).await.unwrap();
        let id = job.id.clone();

        let store1 = Arc::clone(&store);
        let id1 = id.clone();
        let store2 = Arc::clone(&store);
        let id2 = id.clone();

        let t1 = tokio::spawn(async move {
            store1
                .transition(&id1, JobStatus::Queued, JobStatus::Running)
                .await
        });
        let t2 = tokio::spawn(async move {
            store2
                .transition(&id2, JobStatus::Queued, JobStatus::Running)
                .await
        });

        let r1 = t1.await.unwrap();
        let r2 = t2.await.unwrap();

        // Exactly one must succeed, exactly one must get StatusMismatch.
        let successes = [&r1, &r2].iter().filter(|r| r.is_ok()).count();
        let mismatches = [&r1, &r2]
            .iter()
            .filter(|r| matches!(r, Err(JobError::StatusMismatch { .. })))
            .count();

        assert_eq!(successes, 1, "expected exactly one worker to claim the job");
        assert_eq!(mismatches, 1, "expected exactly one StatusMismatch");
    }

    #[tokio::test]
    async fn should_record_the_error_when_a_job_fails() {
        let store = InMemoryJobStore::new();
        let job = store.create(None).await.unwrap();

        store
            .transition(&job.id, JobStatus::Queued, JobStatus::Running)
            .await
            .unwrap();

        let reason = "extraction failed: unsupported format".to_owned();
        let failed = store.fail(&job.id, reason.clone()).await.unwrap();

        assert_eq!(failed.status, JobStatus::Failed);
        assert_eq!(failed.error.as_deref(), Some(reason.as_str()));
        assert!(failed.result_json.is_none());
    }
}
