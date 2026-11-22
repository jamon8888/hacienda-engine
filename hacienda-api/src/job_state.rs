//! API-level policy over the injected [`JobStore`].
//!
//! This module deliberately adds **no persistence mechanism and no durability
//! guarantee**. Durability belongs to the injected [`JobStore`] backend. A host
//! that builds and migrates a Postgres pool can inject
//! `hacienda_core::store::postgres::PostgresJobStore`; the only in-repo runtime
//! constructor, `hacienda serve`, still injects `InMemoryJobStore` and therefore
//! loses queued jobs on restart. Wrapping the seam here keeps two policies that
//! belong to the API boundary, not to a backend:
//!
//! - **Bounded retention.** Terminal jobs are logically expired after a TTL and
//!   physically reclaimed (oldest first) above a per-tenant cap. Only the
//!   content-free fields the store already holds (`status`, timestamps, opaque
//!   counts in `progress_json`/`result_json`, an error *code*) participate —
//!   no message content is read, copied, or written by this module.
//! - **Retry stays disabled.** Re-ingesting an email would need the original
//!   source path, credentials and an idempotency key. Persisting any of those
//!   is forbidden by the transport boundary, and replaying an opaque
//!   `source_ref` without them could duplicate RAG writes under a different
//!   tenant/principal binding. Until that contract exists, a terminal job is
//!   never re-queueable.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use hacienda_core::{
    jobs::{Job, JobError, JobStatus, JobStore},
    tenancy::TenantId,
};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

/// Terminal jobs become invisible to polling after this long in seconds.
/// One day is long enough for a client to collect a result, short enough that
/// an unbounded job history never accumulates.
pub const DEFAULT_TERMINAL_TTL_SECS: i64 = 24 * 60 * 60;

/// Maximum number of terminal jobs retained per tenant. Oldest first is
/// reclaimed when a new job is created.
pub const DEFAULT_MAX_TERMINAL_PER_TENANT: usize = 1_000;

/// A physical prune lists the whole tenant's job history, so it is amortised
/// over this many job writes. Logical expiry on reads is immediate regardless.
pub const DEFAULT_PRUNE_EVERY_WRITES: u64 = 64;

/// Upper bound on distinct tenants tracked by the prune accounting. A tenant
/// arriving while the map is full is pruned immediately rather than clearing
/// the map, so a churn of new tenants can never starve physical reclamation.
const MAX_TRACKED_TENANTS: usize = 4_096;

/// The only retry posture the API supports today.
///
/// `Enabled` is intentionally not a variant: adding it would imply a safe
/// re-ingest path (opaque source ref + tenant/principal binding + idempotency
/// key) that does not exist. See the module doc for the reasoning.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RetryPolicy {
    Disabled,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        RetryPolicy::Disabled
    }
}

/// Retention applied to terminal (`Succeeded` / `Failed`) jobs only. Running or
/// queued jobs are never expired — that would hide work still in flight.
///
/// `prune_every` amortises the cost of reclamation: pruning has to list a whole
/// tenant's jobs (O(n) clones) to find the oldest terminal rows, so doing it on
/// every single job write would make job creation linearly more expensive as a
/// tenant accumulates history. Only once every `prune_every` writes does a
/// physical prune run; expiry is enforced on every read regardless.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct JobRetention {
    pub terminal_ttl_secs: i64,
    pub max_terminal_per_tenant: usize,
    pub prune_every: u64,
}

impl Default for JobRetention {
    fn default() -> Self {
        Self {
            terminal_ttl_secs: DEFAULT_TERMINAL_TTL_SECS,
            max_terminal_per_tenant: DEFAULT_MAX_TERMINAL_PER_TENANT,
            prune_every: DEFAULT_PRUNE_EVERY_WRITES,
        }
    }
}

impl JobRetention {
    /// Invalid values are clamped, never rejected: a retention policy is a
    /// background hygiene concern and must not take down job creation because
    /// of a bad config value.
    pub fn new(terminal_ttl_secs: i64, max_terminal_per_tenant: usize) -> Self {
        Self {
            terminal_ttl_secs: terminal_ttl_secs.max(0),
            // A cap of zero would delete the job that was just finished.
            max_terminal_per_tenant: max_terminal_per_tenant.max(1),
            prune_every: DEFAULT_PRUNE_EVERY_WRITES,
        }
    }

    /// Override the amortisation interval. `0` is clamped to `1` (prune on
    /// every write), which is what the tests use to make reclamation immediate.
    #[must_use]
    pub fn with_prune_every(mut self, prune_every: u64) -> Self {
        self.prune_every = prune_every.max(1);
        self
    }

    fn is_expired(&self, job: &Job, now: DateTime<Utc>) -> bool {
        if !matches!(job.status, JobStatus::Succeeded | JobStatus::Failed) {
            return false;
        }
        // For a terminal job `updated_at` IS the terminal-transition time:
        // `finish`/`fail` set it, and progress writes are no-ops outside
        // `Running` (enforced both here and in the backends). Measuring from
        // `created_at` instead would hide a long-running job the instant it
        // completed if its runtime exceeded the TTL.
        // An unparsable timestamp is treated as *not* expired: fail closed
        // towards keeping data rather than hiding a job on a malformed row.
        let Ok(terminal) = DateTime::parse_from_rfc3339(&job.updated_at) else {
            return false;
        };
        now.signed_duration_since(terminal.with_timezone(&Utc))
            .num_seconds()
            >= self.terminal_ttl_secs
    }
}

/// A `JobStore` decorator that enforces the API's retention and retry policy
/// without owning any storage of its own.
pub struct GuardedJobStore {
    inner: Arc<dyn JobStore>,
    retention: JobRetention,
    retry: RetryPolicy,
    /// Per-tenant write counters so a busy tenant cannot starve another
    /// tenant's pruning. Bounded by [`MAX_TRACKED_TENANTS`].
    prune_writes: Mutex<HashMap<String, u64>>,
}

impl GuardedJobStore {
    /// Wrap a store with the default retention and retry policy.
    pub fn new(inner: Arc<dyn JobStore>) -> Self {
        Self::with_policy(inner, JobRetention::default(), RetryPolicy::Disabled)
    }

    /// Wrap a store with explicit policies (used by the tests below and by
    /// hosts that need a different retention window).
    pub fn with_policy(
        inner: Arc<dyn JobStore>,
        retention: JobRetention,
        retry: RetryPolicy,
    ) -> Self {
        Self {
            inner,
            retention,
            retry,
            prune_writes: Mutex::new(HashMap::new()),
        }
    }

    /// Amortised entry point: only every `prune_every`-th write pays for a
    /// tenant-wide scan. Reads still apply logical expiry immediately.
    async fn maybe_prune(&self, tenant: &TenantId) {
        let writes = {
            // Poison from a panicked writer leaves the map in a valid state;
            // recover rather than turning one panic into permanent no-pruning.
            let mut map = self
                .prune_writes
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            // Evaluate the capacity guard before taking the mutable borrow so
            // the `get_mut` arm below does not overlap with a `map.len()` read.
            let untracked_under_churn =
                !map.contains_key(tenant.as_str()) && map.len() >= MAX_TRACKED_TENANTS;
            match map.get_mut(tenant.as_str()) {
                Some(counter) => {
                    *counter = counter.wrapping_add(1);
                    Some(*counter)
                }
                // Untracked tenant under churn: prune now instead of clearing
                // the map. Clearing would reset every existing tenant's budget
                // and let a stream of new tenants defer pruning indefinitely.
                None if untracked_under_churn => None,
                None => {
                    map.insert(tenant.as_str().to_owned(), 1);
                    Some(1)
                }
            }
        }; // guard dropped before any .await
        if writes.map_or(true, |n| n % self.retention.prune_every == 0) {
            self.prune(tenant).await;
        }
    }

    /// Reclaim expired and over-cap terminal rows for one tenant. Deletion is
    /// best-effort: a backend that does not implement the `delete` seam keeps
    /// the row, but `get`/`list` still hide it via logical expiry.
    async fn prune(&self, tenant: &TenantId) {
        let jobs = match self.inner.list(tenant, None).await {
            Ok(jobs) => jobs,
            Err(e) => {
                tracing::warn!(error = %e, "retention: could not list jobs for pruning");
                return;
            }
        };
        let now = Utc::now();
        let mut to_delete: Vec<String> = jobs
            .iter()
            .filter(|job| self.retention.is_expired(job, now))
            .map(|job| job.id.clone())
            .collect();

        // `list` is oldest-first; counting from the newest end keeps exactly
        // the most recent `max_terminal_per_tenant` terminal jobs.
        let mut kept = 0usize;
        for job in jobs.iter().rev() {
            if !matches!(job.status, JobStatus::Succeeded | JobStatus::Failed) {
                continue;
            }
            kept += 1;
            if kept > self.retention.max_terminal_per_tenant {
                to_delete.push(job.id.clone());
            }
        }

        to_delete.sort_unstable();
        to_delete.dedup();
        for id in to_delete {
            match self.inner.delete(tenant, &id).await {
                Ok(_) => {}
                Err(JobError::DeleteUnsupported(_)) => {
                    tracing::warn!(job_id = %id, "retention: backend has no delete seam; row retained");
                    break;
                }
                Err(e) => tracing::warn!(job_id = %id, error = %e, "retention: delete failed"),
            }
        }
    }
}

#[async_trait]
impl JobStore for GuardedJobStore {
    async fn create(&self, tenant: &TenantId, owner: Option<String>) -> Result<Job, JobError> {
        let job = self.inner.create(tenant, owner).await?;
        self.maybe_prune(tenant).await;
        Ok(job)
    }

    async fn get(&self, tenant: &TenantId, id: &str) -> Result<Option<Job>, JobError> {
        let now = Utc::now();
        Ok(self
            .inner
            .get(tenant, id)
            .await?
            .filter(|job| !self.retention.is_expired(job, now)))
    }

    async fn transition(
        &self,
        tenant: &TenantId,
        id: &str,
        from: JobStatus,
        to: JobStatus,
    ) -> Result<Job, JobError> {
        // A terminal job must never be re-queued. This is the retry policy
        // enforced at the only state-change seam, so no route can opt into a
        // re-ingest by assembling its own transition.
        // Refuse *any* transition out of a terminal state: re-queuing is the
        // obvious retry, but a terminal→terminal CAS would also bump
        // `updated_at` and silently reset the retention clock.
        if self.retry == RetryPolicy::Disabled
            && matches!(from, JobStatus::Succeeded | JobStatus::Failed)
        {
            return Err(JobError::RetryDisabled { id: id.to_owned() });
        }
        self.inner.transition(tenant, id, from, to).await
    }

    async fn finish(
        &self,
        tenant: &TenantId,
        id: &str,
        result_json: String,
    ) -> Result<Job, JobError> {
        // A duplicate finish must not rewrite the terminal timestamp (or the
        // result) — the backends enforce the same predicate atomically.
        let job = match self.inner.get(tenant, id).await? {
            Some(existing)
                if matches!(existing.status, JobStatus::Succeeded | JobStatus::Failed) =>
            {
                existing
            }
            _ => self.inner.finish(tenant, id, result_json).await?,
        };
        self.maybe_prune(tenant).await;
        Ok(job)
    }

    async fn fail(&self, tenant: &TenantId, id: &str, error: String) -> Result<Job, JobError> {
        let job = match self.inner.get(tenant, id).await? {
            Some(existing)
                if matches!(existing.status, JobStatus::Succeeded | JobStatus::Failed) =>
            {
                existing
            }
            _ => self.inner.fail(tenant, id, error).await?,
        };
        self.maybe_prune(tenant).await;
        Ok(job)
    }

    async fn update_progress(
        &self,
        tenant: &TenantId,
        id: &str,
        progress_json: String,
    ) -> Result<Job, JobError> {
        // No extra read here: both backends enforce the Running-only predicate
        // atomically, so the guard can delegate straight through.
        self.inner.update_progress(tenant, id, progress_json).await
    }

    async fn list(
        &self,
        tenant: &TenantId,
        filter: Option<JobStatus>,
    ) -> Result<Vec<Job>, JobError> {
        let now = Utc::now();
        Ok(self
            .inner
            .list(tenant, filter)
            .await?
            .into_iter()
            .filter(|job| !self.retention.is_expired(job, now))
            .collect())
    }

    async fn delete(&self, tenant: &TenantId, id: &str) -> Result<bool, JobError> {
        self.inner.delete(tenant, id).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hacienda_core::jobs::InMemoryJobStore;

    fn t() -> TenantId {
        TenantId::new("tenant-a")
    }

    fn t2() -> TenantId {
        TenantId::new("tenant-b")
    }

    fn inner() -> Arc<dyn JobStore> {
        Arc::new(InMemoryJobStore::new())
    }

    /// A new guard over the *same* repository stands in for an API process
    /// reopening its state after a restart. The durable backend's real reopen
    /// is covered by `hacienda-core`'s ignored Postgres test.
    #[tokio::test]
    async fn reopened_state_reads_jobs_written_by_a_previous_instance() {
        let repo = inner();
        let first = GuardedJobStore::new(Arc::clone(&repo));
        let job = first.create(&t(), Some("alice".to_owned())).await.unwrap();
        first
            .transition(&t(), &job.id, JobStatus::Queued, JobStatus::Running)
            .await
            .unwrap();

        let reopened = GuardedJobStore::new(Arc::clone(&repo));
        let found = reopened
            .get(&t(), &job.id)
            .await
            .unwrap()
            .expect("job must survive a state reopen");
        assert_eq!(found.id, job.id);
        assert_eq!(found.status, JobStatus::Running);
        assert_eq!(found.owner.as_deref(), Some("alice"));
    }

    /// Same principal id in two tenants must not cross the tenant boundary —
    /// the case an owner-only check would miss.
    #[tokio::test]
    async fn tenant_isolation_holds_for_get_list_and_transition() {
        let guard = GuardedJobStore::new(inner());
        let a = guard.create(&t(), Some("alice".to_owned())).await.unwrap();
        let b = guard.create(&t2(), Some("alice".to_owned())).await.unwrap();

        assert!(guard.get(&t2(), &a.id).await.unwrap().is_none());
        assert!(guard.get(&t(), &b.id).await.unwrap().is_none());
        assert_eq!(guard.list(&t(), None).await.unwrap().len(), 1);
        assert_eq!(guard.list(&t2(), None).await.unwrap().len(), 1);
        assert!(matches!(
            guard
                .transition(&t2(), &a.id, JobStatus::Queued, JobStatus::Running)
                .await,
            Err(JobError::NotFound(_))
        ));

        let untouched = guard.get(&t(), &a.id).await.unwrap().unwrap();
        assert_eq!(untouched.status, JobStatus::Queued);
    }

    /// Zero TTL hides a terminal job immediately; the terminal transition
    /// reclaims it through `delete`, and a later create stays idempotent.
    #[tokio::test]
    async fn expired_terminal_jobs_are_hidden_then_reclaimed() {
        let repo = inner();
        let guard = GuardedJobStore::with_policy(
            Arc::clone(&repo),
            JobRetention::new(0, 10).with_prune_every(1),
            RetryPolicy::Disabled,
        );
        let old = guard.create(&t(), None).await.unwrap();
        guard
            .finish(&t(), &old.id, r#"{"count":1}"#.to_owned())
            .await
            .unwrap();

        assert!(guard.get(&t(), &old.id).await.unwrap().is_none());

        guard.create(&t(), None).await.unwrap();
        assert!(
            repo.get(&t(), &old.id).await.unwrap().is_none(),
            "the expired row must be physically reclaimed on the next create"
        );
    }

    /// A running job is never expired, even with a zero TTL.
    #[tokio::test]
    async fn retention_never_expires_non_terminal_jobs() {
        let guard =
            GuardedJobStore::with_policy(inner(), JobRetention::new(0, 10), RetryPolicy::Disabled);
        let job = guard.create(&t(), None).await.unwrap();
        guard
            .transition(&t(), &job.id, JobStatus::Queued, JobStatus::Running)
            .await
            .unwrap();
        assert!(guard.get(&t(), &job.id).await.unwrap().is_some());
    }

    /// The per-tenant cap keeps only the newest terminal jobs.
    #[tokio::test]
    async fn retention_caps_terminal_jobs_per_tenant() {
        let repo = inner();
        let guard = GuardedJobStore::with_policy(
            Arc::clone(&repo),
            JobRetention::new(86_400, 2).with_prune_every(1),
            RetryPolicy::Disabled,
        );
        for _ in 0..3 {
            let job = guard.create(&t(), None).await.unwrap();
            guard.finish(&t(), &job.id, "{}".to_owned()).await.unwrap();
        }
        let terminal = repo
            .list(&t(), None)
            .await
            .unwrap()
            .into_iter()
            .filter(|job| matches!(job.status, JobStatus::Succeeded | JobStatus::Failed))
            .count();
        assert_eq!(terminal, 2, "only the two newest terminal jobs may remain");
    }

    /// Retry is disabled by construction: no terminal job can be re-queued,
    /// and a rejected attempt must not change stored state.
    #[tokio::test]
    async fn retry_policy_refuses_requeue_of_a_failed_job() {
        let repo = inner();
        let guard = GuardedJobStore::new(Arc::clone(&repo));
        assert_eq!(RetryPolicy::default(), RetryPolicy::Disabled);

        let job = guard.create(&t(), None).await.unwrap();
        guard
            .transition(&t(), &job.id, JobStatus::Queued, JobStatus::Running)
            .await
            .unwrap();
        guard
            .fail(
                &t(),
                &job.id,
                "email processing failed (backend)".to_owned(),
            )
            .await
            .unwrap();

        for to in [JobStatus::Queued, JobStatus::Running, JobStatus::Succeeded] {
            assert!(matches!(
                guard.transition(&t(), &job.id, JobStatus::Failed, to).await,
                Err(JobError::RetryDisabled { .. })
            ));
        }
        assert_eq!(
            repo.get(&t(), &job.id).await.unwrap().unwrap().status,
            JobStatus::Failed,
            "a refused retry must not mutate the job"
        );
    }

    /// Invalid configuration is clamped rather than allowed to disable
    /// retention entirely or delete the job that just completed.
    #[test]
    fn invalid_ttl_cap_and_prune_interval_are_clamped() {
        let retention = JobRetention::new(-5, 0).with_prune_every(0);
        assert_eq!(retention.terminal_ttl_secs, 0);
        assert_eq!(retention.max_terminal_per_tenant, 1);
        assert_eq!(retention.prune_every, 1);
    }

    /// `updated_at` on a terminal job is its terminal-transition time, not the
    /// time of a late progress tick. Once that much time has passed the job
    /// expires, regardless of how long it took to finish.
    #[test]
    fn progress_updates_do_not_reset_retention_age() {
        let retention = JobRetention::new(60, 10);
        let now = chrono::Utc::now();
        let job = Job {
            id: "job-1".to_owned(),
            status: JobStatus::Failed,
            created_at: (now - chrono::Duration::seconds(180)).to_rfc3339(),
            updated_at: (now - chrono::Duration::seconds(120)).to_rfc3339(),
            result_json: None,
            error: None,
            progress_json: None,
            owner: None,
            tenant_id: t().to_string(),
        };
        assert!(
            retention.is_expired(&job, now),
            "terminal age is measured from the finish/fail time"
        );
    }

    /// Regression for the MEDIUM finding: a job that ran longer than the TTL
    /// must remain pollable for a full TTL after it completes, then expire.
    #[test]
    fn long_running_job_stays_visible_for_ttl_after_completion() {
        let retention = JobRetention::new(60, 10);
        let now = chrono::Utc::now();
        let job = Job {
            id: "job-long".to_owned(),
            status: JobStatus::Succeeded,
            created_at: (now - chrono::Duration::seconds(600)).to_rfc3339(),
            updated_at: (now - chrono::Duration::seconds(10)).to_rfc3339(),
            result_json: Some("{}".to_owned()),
            error: None,
            progress_json: None,
            owner: None,
            tenant_id: t().to_string(),
        };
        assert!(
            !retention.is_expired(&job, now),
            "a 10-minute run finishing 10s ago must still be visible"
        );
        assert!(
            !retention.is_expired(&job, now + chrono::Duration::seconds(49)),
            "still visible just before the terminal TTL elapses"
        );
        assert!(
            retention.is_expired(&job, now + chrono::Duration::seconds(60)),
            "expires one terminal TTL after completion"
        );
    }

    /// A late progress tick through the guard must not extend a terminal job's
    /// visible lifetime.
    #[tokio::test]
    async fn late_progress_tick_does_not_renew_terminal_age() {
        let guard = GuardedJobStore::new(inner());
        let job = guard.create(&t(), None).await.unwrap();
        guard
            .transition(&t(), &job.id, JobStatus::Queued, JobStatus::Running)
            .await
            .unwrap();
        let finished = guard.finish(&t(), &job.id, "{}".to_owned()).await.unwrap();
        let after = guard
            .update_progress(&t(), &job.id, r#"{"late":true}"#.to_owned())
            .await
            .unwrap();
        assert_eq!(after.updated_at, finished.updated_at);
        assert_eq!(after.progress_json, finished.progress_json);
    }

    /// A repeated terminal write through the guard must preserve the original
    /// terminal timestamp, otherwise a TTL-expired job could be revived.
    #[tokio::test]
    async fn duplicate_terminal_writes_through_guard_keep_terminal_time() {
        let guard = GuardedJobStore::with_policy(
            inner(),
            JobRetention::new(86_400, 10).with_prune_every(64),
            RetryPolicy::Disabled,
        );
        let succeeded = guard.create(&t(), None).await.unwrap();
        let first = guard
            .finish(&t(), &succeeded.id, "{}".to_owned())
            .await
            .unwrap();
        let again = guard
            .finish(&t(), &succeeded.id, "{}".to_owned())
            .await
            .unwrap();
        assert_eq!(again.updated_at, first.updated_at);

        let failed = guard.create(&t(), None).await.unwrap();
        let first_fail = guard
            .fail(&t(), &failed.id, "first".to_owned())
            .await
            .unwrap();
        let again = guard
            .fail(&t(), &failed.id, "second".to_owned())
            .await
            .unwrap();
        assert_eq!(again.updated_at, first_fail.updated_at);
        assert_eq!(again.error.as_deref(), Some("first"));
    }

    /// The tenant-wide prune is amortised: it does not run on every write.
    /// Logical expiry still applies immediately on reads.
    #[tokio::test]
    async fn pruning_is_amortized_not_per_write() {
        let repo = inner();
        let guard = GuardedJobStore::with_policy(
            Arc::clone(&repo),
            JobRetention::new(86_400, 1).with_prune_every(64),
            RetryPolicy::Disabled,
        );
        for _ in 0..3 {
            let job = guard.create(&t(), None).await.unwrap();
            guard.finish(&t(), &job.id, "{}".to_owned()).await.unwrap();
        }
        let terminal = repo
            .list(&t(), None)
            .await
            .unwrap()
            .into_iter()
            .filter(|job| matches!(job.status, JobStatus::Succeeded | JobStatus::Failed))
            .count();
        assert_eq!(terminal, 3, "a physical prune must not run on every write");
    }

    /// A counting backend makes the amortisation visible: a busy tenant's
    /// writes must not consume another tenant's prune budget.
    struct CountingStore {
        inner: Arc<dyn JobStore>,
        list_calls: Arc<std::sync::atomic::AtomicU64>,
    }

    #[async_trait]
    impl JobStore for CountingStore {
        async fn create(&self, tenant: &TenantId, owner: Option<String>) -> Result<Job, JobError> {
            self.inner.create(tenant, owner).await
        }
        async fn get(&self, tenant: &TenantId, id: &str) -> Result<Option<Job>, JobError> {
            self.inner.get(tenant, id).await
        }
        async fn transition(
            &self,
            tenant: &TenantId,
            id: &str,
            from: JobStatus,
            to: JobStatus,
        ) -> Result<Job, JobError> {
            self.inner.transition(tenant, id, from, to).await
        }
        async fn finish(
            &self,
            tenant: &TenantId,
            id: &str,
            result_json: String,
        ) -> Result<Job, JobError> {
            self.inner.finish(tenant, id, result_json).await
        }
        async fn fail(&self, tenant: &TenantId, id: &str, error: String) -> Result<Job, JobError> {
            self.inner.fail(tenant, id, error).await
        }
        async fn update_progress(
            &self,
            tenant: &TenantId,
            id: &str,
            progress_json: String,
        ) -> Result<Job, JobError> {
            self.inner.update_progress(tenant, id, progress_json).await
        }
        async fn list(
            &self,
            tenant: &TenantId,
            filter: Option<JobStatus>,
        ) -> Result<Vec<Job>, JobError> {
            self.list_calls
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            self.inner.list(tenant, filter).await
        }
        async fn delete(&self, tenant: &TenantId, id: &str) -> Result<bool, JobError> {
            self.inner.delete(tenant, id).await
        }
    }

    #[tokio::test]
    async fn prune_accounting_is_per_tenant() {
        use std::sync::atomic::Ordering;
        let list_calls = Arc::new(std::sync::atomic::AtomicU64::new(0));
        let repo: Arc<dyn JobStore> = Arc::new(CountingStore {
            inner: inner(),
            list_calls: Arc::clone(&list_calls),
        });
        let guard = GuardedJobStore::with_policy(
            repo,
            JobRetention::new(86_400, 100).with_prune_every(4),
            RetryPolicy::Disabled,
        );

        for _ in 0..3 {
            guard.create(&t(), None).await.unwrap();
        }
        guard.create(&t2(), None).await.unwrap();
        assert_eq!(
            list_calls.load(Ordering::Relaxed),
            0,
            "tenant B's first write must not inherit tenant A's counter"
        );

        for _ in 0..3 {
            guard.create(&t2(), None).await.unwrap();
        }
        assert_eq!(
            list_calls.load(Ordering::Relaxed),
            1,
            "tenant B's own fourth write triggers exactly one prune"
        );
    }

    /// More tenants than the counter map can hold: the untracked tenant is
    /// pruned immediately, and existing tenants keep their counters (no
    /// wholesale `clear` that would starve reclamation under churn).
    #[tokio::test]
    async fn tenant_churn_beyond_map_cap_does_not_starve_pruning() {
        use std::sync::atomic::Ordering;
        let list_calls = Arc::new(std::sync::atomic::AtomicU64::new(0));
        let repo: Arc<dyn JobStore> = Arc::new(CountingStore {
            inner: inner(),
            list_calls: Arc::clone(&list_calls),
        });
        let guard = GuardedJobStore::with_policy(
            repo,
            JobRetention::new(86_400, 100).with_prune_every(2),
            RetryPolicy::Disabled,
        );

        for i in 0..MAX_TRACKED_TENANTS {
            guard
                .create(&TenantId::new(format!("tenant-{i}")), None)
                .await
                .unwrap();
        }
        assert_eq!(list_calls.load(Ordering::Relaxed), 0);

        guard
            .create(&TenantId::new("tenant-churn"), None)
            .await
            .unwrap();
        assert_eq!(
            list_calls.load(Ordering::Relaxed),
            1,
            "an untracked tenant at map capacity must trigger an immediate prune"
        );

        guard
            .create(&TenantId::new("tenant-0"), None)
            .await
            .unwrap();
        assert_eq!(
            list_calls.load(Ordering::Relaxed),
            2,
            "a tracked tenant's counter must survive the churn"
        );
    }
}
