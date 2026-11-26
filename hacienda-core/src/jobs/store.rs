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
use crate::tenancy::TenantId;
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
#[allow(clippy::double_must_use)]
#[async_trait]
pub trait JobStore: Send + Sync {
    /// Create a new job in the `Queued` state, owned by `tenant`.
    ///
    /// `owner` is the principal id of the caller who submitted the job. Pass
    /// `None` for trusted in-process callers (CLI, desktop app). The transport
    /// layer uses `owner` to enforce **actor-level** isolation — a principal who
    /// requests a job they do not own receives 404 rather than 403 so that the
    /// existence of another actor's job is not disclosed (OWASP A01).
    ///
    /// `tenant` is a second, independent dimension (S1b): it answers "which
    /// customer's data did touching this job disclose," not "whose job is this."
    /// An org's admin (`Caller::Trusted` or a privileged principal) has no way to see
    /// their own org's jobs collectively through `owner` alone — every owner-less job
    /// looked identical regardless of which tenant it belonged to before this field
    /// existed. `owner` and `tenant` are checked independently; neither supersedes the
    /// other. See `tenancy.rs`'s module doc (D-S1-1) and this crate's S1b spec §1.3.
    ///
    /// The store records both values without enforcing `owner`; actor-level
    /// enforcement belongs at the transport layer where the HTTP status code is
    /// chosen. `tenant` **is** enforced by the store itself on every other method
    /// below — a job never leaks across the `&TenantId` boundary the way it could
    /// leak across `owner` if a caller forgot the transport-layer check.
    async fn create(&self, tenant: &TenantId, owner: Option<String>) -> Result<Job, JobError>;

    /// Retrieve a job by id, returning `None` if it does not exist **or belongs to a
    /// different tenant** — the two are indistinguishable to the caller (D-S1b-1: a
    /// cross-tenant id is reported not-found, never forbidden, since a distinguishable
    /// error would itself disclose that the id is valid somewhere).
    async fn get(&self, tenant: &TenantId, id: &str) -> Result<Option<Job>, JobError>;

    /// Atomically transition a job's status from `from` to `to`.
    ///
    /// Fails with [`JobError::StatusMismatch`] if the job's current status is not
    /// `from`. This compare-and-swap is what stops two workers both claiming the same
    /// queued job: only the one whose `from` matches the stored status wins.
    async fn transition(&self, tenant: &TenantId, id: &str, from: JobStatus, to: JobStatus) -> Result<Job, JobError>;

    /// Mark a job as `Succeeded` and record its serialised result payload.
    ///
    /// Backends MUST make the first terminal write win **atomically**: apply the
    /// update only when the stored status is neither `Succeeded` nor `Failed`,
    /// and otherwise return the stored job unchanged (status, result and
    /// `updated_at` all preserved). A read-then-write check is not enough — two
    /// workers can both observe a non-terminal row before either writes. The
    /// in-repo stores enforce this under their lock / row-level status
    /// predicate. `updated_at` is the retention clock for terminal jobs, so a
    /// duplicate write that refreshes it would extend retention indefinitely.
    async fn finish(&self, tenant: &TenantId, id: &str, result_json: String) -> Result<Job, JobError>;

    /// Mark a job as `Failed` and record the reason.
    ///
    /// The same atomic first-terminal-write rule as [`Self::finish`] applies:
    /// write only if the stored status is not already `Succeeded` or `Failed`,
    /// otherwise return the stored job unchanged. This includes preserving the
    /// original reason and `updated_at` on a duplicate `fail` or on a `fail`
    /// racing a `finish`.
    async fn fail(&self, tenant: &TenantId, id: &str, reason: String) -> Result<Job, JobError>;

    /// Overwrite a running job's progress payload.
    ///
    /// A separate verb from `transition`/`finish`/`fail` because progress
    /// updates happen many times over a job's `Running` lifetime and carry no
    /// status change — folding them into `transition`'s compare-and-swap
    /// would force every progress tick to also re-assert `Running == Running`,
    /// racing against a concurrent `finish`/`fail` for no benefit.
    ///
    /// Only a `Running` job accepts a progress update. A late tick racing a
    /// terminal transition is a no-op that returns the current job, not an
    /// error — so a worker still cannot fail because of a race it cannot
    /// observe. Critically, a no-op tick must not advance `updated_at`: for a
    /// terminal job that field is the terminal-transition time used to measure
    /// retention age, and letting a late tick overwrite it would make a
    /// completed job stay visible indefinitely.
    async fn update_progress(&self, tenant: &TenantId, id: &str, progress_json: String) -> Result<Job, JobError>;

    /// List every job belonging to `tenant`, optionally filtered by status. Ordered by
    /// creation time, oldest first.
    async fn list(&self, tenant: &TenantId, filter: Option<JobStatus>) -> Result<Vec<Job>, JobError>;

    /// Permanently remove one job. Optional so existing backends keep compiling:
    /// the default is a typed "unsupported" error, and a retention policy that
    /// calls it simply logs the warning instead of failing job creation.
    ///
    /// Implementations MUST scope the delete by `tenant` as well as `id` — a
    /// cross-tenant delete must be a silent no-op, not a distinguishable error.
    /// Returns `true` when a row was actually removed.
    async fn delete(&self, tenant: &TenantId, id: &str) -> Result<bool, JobError> {
        let _ = (tenant, id);
        Err(JobError::DeleteUnsupported(std::any::type_name::<Self>().into()))
    }
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

/// Build a context-rich error for a poisoned in-memory lock.
///
/// Carries the operation, the tenant/job identifiers, the root cause and the
/// recovery step. Job payloads (`result_json`/`error`/`progress_json`) are
/// deliberately absent — they can hold customer data and are never needed to
/// diagnose a mutex poison.
fn lock_poisoned(operation: &str, tenant: &TenantId, id: &str) -> JobError {
    JobError::Internal(format!(
        "{operation}: job {id} (tenant {tenant}): root cause: the jobs mutex was poisoned \
         by a panic in a previous holder; next step: recreate the store (restart the \
         process) because poisoned in-memory state cannot be reused safely",
        tenant = tenant.as_str()
    ))
}

#[async_trait]
impl JobStore for InMemoryJobStore {
    async fn create(&self, tenant: &TenantId, owner: Option<String>) -> Result<Job, JobError> {
        let id = Uuid::new_v4().to_string();
        let now = now_rfc3339();
        let job = Job {
            id: id.clone(),
            status: JobStatus::Queued,
            created_at: now.clone(),
            updated_at: now,
            result_json: None,
            error: None,
            progress_json: None,
            owner,
            tenant_id: tenant.to_string(),
        };
        // Take the guard, insert, drop — no .await while held.
        let mut guard = self.jobs.lock().map_err(|_| lock_poisoned("create", tenant, &id))?;
        guard.insert(id, job.clone());
        Ok(job)
    }

    async fn get(&self, tenant: &TenantId, id: &str) -> Result<Option<Job>, JobError> {
        let guard = self.jobs.lock().map_err(|_| lock_poisoned("get", tenant, id))?;
        // Matching on id && tenant together, rather than looking up by id and checking
        // tenant after, means a cross-tenant id falls straight into the same `None` a
        // nonexistent id produces — the not-found-not-forbidden property (D-S1b-1) falls
        // out of the lookup itself.
        Ok(guard.get(id).filter(|j| j.tenant_id == tenant.as_str()).cloned())
    }

    async fn transition(&self, tenant: &TenantId, id: &str, from: JobStatus, to: JobStatus) -> Result<Job, JobError> {
        let mut guard = self.jobs.lock().map_err(|_| lock_poisoned("transition", tenant, id))?;
        let job = guard
            .get_mut(id)
            .filter(|j| j.tenant_id == tenant.as_str())
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

    async fn finish(&self, tenant: &TenantId, id: &str, result_json: String) -> Result<Job, JobError> {
        let mut guard = self.jobs.lock().map_err(|_| lock_poisoned("finish", tenant, id))?;
        let job = guard
            .get_mut(id)
            .filter(|j| j.tenant_id == tenant.as_str())
            .ok_or_else(|| JobError::NotFound(id.to_owned()))?;
        // A duplicate terminal write is a no-op: `updated_at` is the terminal
        // transition time used for retention and must not be refreshed.
        if matches!(job.status, JobStatus::Succeeded | JobStatus::Failed) {
            return Ok(job.clone());
        }
        job.status = JobStatus::Succeeded;
        job.result_json = Some(result_json);
        job.updated_at = now_rfc3339();
        Ok(job.clone())
    }

    async fn fail(&self, tenant: &TenantId, id: &str, reason: String) -> Result<Job, JobError> {
        let mut guard = self.jobs.lock().map_err(|_| lock_poisoned("fail", tenant, id))?;
        let job = guard
            .get_mut(id)
            .filter(|j| j.tenant_id == tenant.as_str())
            .ok_or_else(|| JobError::NotFound(id.to_owned()))?;
        if matches!(job.status, JobStatus::Succeeded | JobStatus::Failed) {
            return Ok(job.clone());
        }
        job.status = JobStatus::Failed;
        job.error = Some(reason);
        job.updated_at = now_rfc3339();
        Ok(job.clone())
    }

    async fn update_progress(&self, tenant: &TenantId, id: &str, progress_json: String) -> Result<Job, JobError> {
        let mut guard = self
            .jobs
            .lock()
            .map_err(|_| lock_poisoned("update_progress", tenant, id))?;
        let job = guard
            .get_mut(id)
            .filter(|j| j.tenant_id == tenant.as_str())
            .ok_or_else(|| JobError::NotFound(id.to_owned()))?;
        // A late tick from a worker that lost a race to finish/fail must not
        // move `updated_at` (the terminal timestamp) or overwrite progress.
        if job.status != JobStatus::Running {
            return Ok(job.clone());
        }
        job.progress_json = Some(progress_json);
        job.updated_at = now_rfc3339();
        Ok(job.clone())
    }

    async fn list(&self, tenant: &TenantId, filter: Option<JobStatus>) -> Result<Vec<Job>, JobError> {
        // `list` is tenant-wide, so there is no single job id to report; the
        // sentinel keeps the error's job context explicit rather than blank.
        let guard = self.jobs.lock().map_err(|_| lock_poisoned("list", tenant, "<all>"))?;
        let mut jobs: Vec<Job> = guard
            .values()
            .filter(|j| j.tenant_id == tenant.as_str() && filter.map(|status| j.status == status).unwrap_or(true))
            .cloned()
            .collect();
        // Stable order: oldest created_at first, id as tie-break. The tie-break is not
        // decoration — RFC 3339 timestamps have finite resolution, so jobs created in the
        // same instant compare equal and would otherwise come back in `HashMap` iteration
        // order, which is randomised per process.
        jobs.sort_by(|a, b| a.created_at.cmp(&b.created_at).then_with(|| a.id.cmp(&b.id)));
        Ok(jobs)
    }

    async fn delete(&self, tenant: &TenantId, id: &str) -> Result<bool, JobError> {
        let mut guard = self.jobs.lock().map_err(|_| lock_poisoned("delete", tenant, id))?;
        // Check tenant *before* removing: `HashMap::remove` would otherwise
        // delete another tenant's row before the filter could reject it. A
        // cross-tenant delete is a silent no-op, indistinguishable from a
        // missing id (D-S1b-1).
        let belongs_to_tenant = guard.get(id).is_some_and(|j| j.tenant_id == tenant.as_str());
        if !belongs_to_tenant {
            return Ok(false);
        }
        Ok(guard.remove(id).is_some())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The tenant used by every test that doesn't itself care about tenant scoping.
    fn t() -> TenantId {
        TenantId::new("tenant-a")
    }

    /// A second, distinct tenant — used only by the cross-tenant isolation tests below.
    fn t2() -> TenantId {
        TenantId::new("tenant-b")
    }

    #[tokio::test]
    async fn should_create_a_job_in_the_queued_state() {
        let store = InMemoryJobStore::new();
        let job = store.create(&t(), None).await.unwrap();
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
        let job = store.create(&t(), Some("alice".into())).await.unwrap();
        assert_eq!(job.owner.as_deref(), Some("alice"));

        // get round-trips the owner unchanged
        let fetched = store.get(&t(), &job.id).await.unwrap().unwrap();
        assert_eq!(fetched.owner.as_deref(), Some("alice"));
    }

    #[tokio::test]
    async fn should_stamp_the_creating_tenant_onto_the_job() {
        let store = InMemoryJobStore::new();
        let acme = TenantId::new("acme");
        let job = store.create(&acme, None).await.unwrap();
        assert_eq!(job.tenant_id, "acme");

        let fetched = store.get(&acme, &job.id).await.unwrap().unwrap();
        assert_eq!(fetched.tenant_id, "acme");
    }

    #[tokio::test]
    async fn should_accept_no_owner_for_trusted_callers() {
        let store = InMemoryJobStore::new();
        let job = store.create(&t(), None).await.unwrap();
        assert!(job.owner.is_none());

        let fetched = store.get(&t(), &job.id).await.unwrap().unwrap();
        assert!(fetched.owner.is_none());
    }

    #[tokio::test]
    async fn should_refuse_a_transition_from_an_unexpected_status() {
        let store = InMemoryJobStore::new();
        let job = store.create(&t(), None).await.unwrap();

        // Trying to transition from Running when it's actually Queued must fail.
        let err = store
            .transition(&t(), &job.id, JobStatus::Running, JobStatus::Succeeded)
            .await
            .unwrap_err();

        match err {
            JobError::StatusMismatch { id, expected, actual } => {
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
        let job = store.create(&t(), None).await.unwrap();
        let id = job.id.clone();

        let store1 = Arc::clone(&store);
        let id1 = id.clone();
        let store2 = Arc::clone(&store);
        let id2 = id.clone();

        let t1 = tokio::spawn(async move {
            store1
                .transition(&t(), &id1, JobStatus::Queued, JobStatus::Running)
                .await
        });
        let t2 = tokio::spawn(async move {
            store2
                .transition(&t(), &id2, JobStatus::Queued, JobStatus::Running)
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
        let job = store.create(&t(), None).await.unwrap();

        store
            .transition(&t(), &job.id, JobStatus::Queued, JobStatus::Running)
            .await
            .unwrap();

        let reason = "extraction failed: unsupported format".to_owned();
        let failed = store.fail(&t(), &job.id, reason.clone()).await.unwrap();

        assert_eq!(failed.status, JobStatus::Failed);
        assert_eq!(failed.error.as_deref(), Some(reason.as_str()));
        assert!(failed.result_json.is_none());
    }

    #[tokio::test]
    async fn should_overwrite_progress_when_update_progress_repeats() {
        let store = InMemoryJobStore::new();
        let job = store.create(&t(), None).await.unwrap();
        store
            .transition(&t(), &job.id, JobStatus::Queued, JobStatus::Running)
            .await
            .unwrap();

        let updated = store
            .update_progress(&t(), &job.id, r#"{"done":1,"total":10}"#.to_string())
            .await
            .unwrap();
        assert_eq!(updated.progress_json.as_deref(), Some(r#"{"done":1,"total":10}"#));

        let updated = store
            .update_progress(&t(), &job.id, r#"{"done":5,"total":10}"#.to_string())
            .await
            .unwrap();
        assert_eq!(updated.progress_json.as_deref(), Some(r#"{"done":5,"total":10}"#));
    }

    #[tokio::test]
    async fn should_fail_update_progress_when_job_is_unknown() {
        let store = InMemoryJobStore::new();
        let err = store
            .update_progress(&t(), "missing", "{}".to_string())
            .await
            .unwrap_err();
        assert!(matches!(err, JobError::NotFound(id) if id == "missing"));
    }

    /// D-S1b-1: an id that belongs to a different tenant must be reported exactly like an
    /// id that does not exist at all — `Ok(None)`, never a distinguishable error.
    #[tokio::test]
    async fn job_get_for_another_tenants_id_is_not_found() {
        let store = InMemoryJobStore::new();
        let job = store.create(&t(), None).await.unwrap();

        assert!(store.get(&t(), &job.id).await.unwrap().is_some());
        assert!(store.get(&t2(), &job.id).await.unwrap().is_none());
    }

    /// Same not-found-not-forbidden discipline applies to the compare-and-swap methods:
    /// a cross-tenant `transition`/`finish`/`fail`/`update_progress` must fail with
    /// `NotFound`, not a status-shaped error that would confirm the id exists elsewhere.
    #[tokio::test]
    async fn job_mutations_for_another_tenants_id_are_not_found() {
        let store = InMemoryJobStore::new();
        let job = store.create(&t(), None).await.unwrap();

        assert!(matches!(
            store
                .transition(&t2(), &job.id, JobStatus::Queued, JobStatus::Running)
                .await,
            Err(JobError::NotFound(id)) if id == job.id
        ));
        assert!(matches!(
            store.finish(&t2(), &job.id, "{}".to_string()).await,
            Err(JobError::NotFound(id)) if id == job.id
        ));
        assert!(matches!(
            store.fail(&t2(), &job.id, "boom".to_string()).await,
            Err(JobError::NotFound(id)) if id == job.id
        ));
        assert!(matches!(
            store.update_progress(&t2(), &job.id, "{}".to_string()).await,
            Err(JobError::NotFound(id)) if id == job.id
        ));

        // Tenant a's job is untouched by tenant b's rejected attempts.
        let job = store.get(&t(), &job.id).await.unwrap().unwrap();
        assert_eq!(job.status, JobStatus::Queued);
    }

    /// Regression guard (S1b spec §5): adding `tenant_id` must not weaken the existing
    /// actor-level `owner` check. Two different owners within the *same* tenant are both
    /// visible through the store's tenant-scoped `list` — owner-level filtering is, and
    /// remains, the transport layer's job (see `hacienda-api/src/handlers/jobs.rs`), not
    /// something `tenant` scoping should accidentally take over.
    #[tokio::test]
    async fn job_store_owner_isolation_still_holds_within_a_tenant() {
        let store = InMemoryJobStore::new();
        store.create(&t(), Some("alice".into())).await.unwrap();
        store.create(&t(), Some("bob".into())).await.unwrap();

        let jobs = store.list(&t(), None).await.unwrap();
        assert_eq!(jobs.len(), 2, "tenant-level list sees both owners' jobs");
        let owners: Vec<_> = jobs.iter().filter_map(|j| j.owner.as_deref()).collect();
        assert!(owners.contains(&"alice"));
        assert!(owners.contains(&"bob"));

        // A different tenant sees neither, regardless of owner.
        let other_tenant_jobs = store.list(&t2(), None).await.unwrap();
        assert!(other_tenant_jobs.is_empty());
    }

    /// The retention seam must never let one tenant reclaim another tenant's row.
    #[tokio::test]
    async fn delete_is_tenant_scoped_and_idempotent() {
        let store = InMemoryJobStore::new();
        let job = store.create(&t(), None).await.unwrap();

        assert!(!store.delete(&t2(), &job.id).await.unwrap());
        assert!(
            store.get(&t(), &job.id).await.unwrap().is_some(),
            "a rejected cross-tenant delete must leave the row untouched"
        );

        assert!(store.delete(&t(), &job.id).await.unwrap());
        assert!(!store.delete(&t(), &job.id).await.unwrap());
        assert!(store.get(&t(), &job.id).await.unwrap().is_none());
    }

    /// A progress tick that arrives after `finish` is ignored. Its `updated_at`
    /// is the terminal-transition time used for retention, so a late tick must
    /// not move it or revive the job's visible lifetime.
    #[tokio::test]
    async fn progress_after_terminal_is_a_no_op_that_keeps_terminal_time() {
        let store = InMemoryJobStore::new();
        let job = store.create(&t(), None).await.unwrap();
        store
            .transition(&t(), &job.id, JobStatus::Queued, JobStatus::Running)
            .await
            .unwrap();
        let finished = store.finish(&t(), &job.id, "{}".to_owned()).await.unwrap();
        let terminal_time = finished.updated_at.clone();

        let after = store
            .update_progress(&t(), &job.id, r#"{"late":true}"#.to_owned())
            .await
            .unwrap();
        assert_eq!(after.status, JobStatus::Succeeded);
        assert_eq!(after.updated_at, terminal_time);
        assert_eq!(after.progress_json, finished.progress_json);
    }

    /// Duplicate terminal writes must not refresh the retention timestamp or
    /// overwrite the first terminal result/reason.
    #[tokio::test]
    async fn duplicate_finish_and_fail_keep_the_first_terminal_time() {
        let store = InMemoryJobStore::new();

        let succeeded = store.create(&t(), None).await.unwrap();
        let first = store
            .finish(&t(), &succeeded.id, r#"{"first":true}"#.to_owned())
            .await
            .unwrap();
        let again = store
            .finish(&t(), &succeeded.id, r#"{"second":true}"#.to_owned())
            .await
            .unwrap();
        assert_eq!(again.updated_at, first.updated_at);
        assert_eq!(again.result_json, first.result_json);
        assert_eq!(again.status, JobStatus::Succeeded);

        let failed = store.create(&t(), None).await.unwrap();
        let first_fail = store.fail(&t(), &failed.id, "first".to_owned()).await.unwrap();
        let again = store.fail(&t(), &failed.id, "second".to_owned()).await.unwrap();
        assert_eq!(again.updated_at, first_fail.updated_at);
        assert_eq!(again.error.as_deref(), Some("first"));
        assert_eq!(again.status, JobStatus::Failed);

        // A failed job cannot be overwritten by a later finish either.
        let cross = store.finish(&t(), &failed.id, "{}".to_owned()).await.unwrap();
        assert_eq!(cross.updated_at, first_fail.updated_at);
        assert_eq!(cross.status, JobStatus::Failed);
    }

    /// Both the first terminal write and a duplicate must return the progress
    /// payload written while the job was `Running`, alongside the original
    /// result/reason and status. A terminal response with `progress_json: None`
    /// would hide the last progress tick from a poller reading the same job.
    #[tokio::test]
    async fn terminal_writes_keep_progress_json_and_result() {
        let store = InMemoryJobStore::new();
        let progress = r#"{"page":3,"chunks":9}"#.to_owned();

        let succeeded = store.create(&t(), None).await.unwrap();
        store
            .transition(&t(), &succeeded.id, JobStatus::Queued, JobStatus::Running)
            .await
            .unwrap();
        store
            .update_progress(&t(), &succeeded.id, progress.clone())
            .await
            .unwrap();

        let first = store
            .finish(&t(), &succeeded.id, r#"{"ok":true}"#.to_owned())
            .await
            .unwrap();
        assert_eq!(first.status, JobStatus::Succeeded);
        assert_eq!(first.result_json.as_deref(), Some(r#"{"ok":true}"#));
        assert_eq!(first.progress_json.as_deref(), Some(progress.as_str()));

        let again = store
            .finish(&t(), &succeeded.id, r#"{"ok":false}"#.to_owned())
            .await
            .unwrap();
        assert_eq!(again.status, JobStatus::Succeeded);
        assert_eq!(again.result_json.as_deref(), Some(r#"{"ok":true}"#));
        assert_eq!(again.progress_json.as_deref(), Some(progress.as_str()));

        let failed = store.create(&t(), None).await.unwrap();
        store
            .transition(&t(), &failed.id, JobStatus::Queued, JobStatus::Running)
            .await
            .unwrap();
        store.update_progress(&t(), &failed.id, progress.clone()).await.unwrap();

        let first_fail = store.fail(&t(), &failed.id, "first".to_owned()).await.unwrap();
        assert_eq!(first_fail.status, JobStatus::Failed);
        assert_eq!(first_fail.error.as_deref(), Some("first"));
        assert_eq!(first_fail.progress_json.as_deref(), Some(progress.as_str()));

        let again = store.fail(&t(), &failed.id, "second".to_owned()).await.unwrap();
        assert_eq!(again.status, JobStatus::Failed);
        assert_eq!(again.error.as_deref(), Some("first"));
        assert_eq!(again.progress_json.as_deref(), Some(progress.as_str()));
    }

    /// Every in-memory lock site must return a contextual error, not a bare
    /// "lock poisoned" string. Poison the mutex once on a throwaway clone,
    /// then assert operation/tenant/job/root-cause/recovery on all eight verbs.
    #[tokio::test]
    async fn all_lock_sites_report_contextual_poison_errors() {
        let store = Arc::new(InMemoryJobStore::new());
        let job = store.create(&t(), None).await.unwrap();
        let id = job.id.clone();
        // Give the job a distinctive payload; it must not appear in any error.
        let secret = "CUSTOMER-PAYLOAD-MUST-NOT-LEAK";
        store
            .transition(&t(), &id, JobStatus::Queued, JobStatus::Running)
            .await
            .unwrap();
        store.update_progress(&t(), &id, secret.to_owned()).await.unwrap();

        let poison_target = Arc::clone(&store);
        let _ = std::thread::spawn(move || {
            let _guard = poison_target.jobs.lock().unwrap();
            panic!("poison the jobs mutex on purpose");
        })
        .join();

        // `create` and `list` are not addressed by a caller-supplied job id,
        // so only the other six sites assert an echoed id.
        let checks = [
            ("create", store.create(&t(), None).await.err(), None),
            ("get", store.get(&t(), &id).await.err(), Some(id.as_str())),
            (
                "transition",
                store
                    .transition(&t(), &id, JobStatus::Queued, JobStatus::Running)
                    .await
                    .err(),
                Some(id.as_str()),
            ),
            (
                "finish",
                store.finish(&t(), &id, "{}".to_owned()).await.err(),
                Some(id.as_str()),
            ),
            (
                "fail",
                store.fail(&t(), &id, "boom".to_owned()).await.err(),
                Some(id.as_str()),
            ),
            (
                "update_progress",
                store.update_progress(&t(), &id, "{}".to_owned()).await.err(),
                Some(id.as_str()),
            ),
            ("list", store.list(&t(), None).await.err(), None),
            ("delete", store.delete(&t(), &id).await.err(), Some(id.as_str())),
        ];

        for (operation, err, job_id) in checks {
            let err = err.expect("a poisoned store must reject every operation");
            let message = err.to_string();
            assert!(message.contains(operation), "operation missing: {message}");
            assert!(message.contains(t().as_str()), "tenant missing: {message}");
            if let Some(job_id) = job_id {
                assert!(message.contains(job_id), "job id missing: {message}");
            }
            assert!(message.contains("panic"), "root cause missing: {message}");
            assert!(message.contains("restart"), "recovery step missing: {message}");
            assert!(!message.contains(secret), "payload leaked into error: {message}");
        }
    }
}
