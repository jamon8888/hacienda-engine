//! Postgres [`JobStore`] implementation.

use crate::jobs::{
    error::JobError,
    types::{Job, JobStatus},
    JobStore,
};
use crate::tenancy::TenantId;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sqlx::PgPool;
use uuid::Uuid;

/// Postgres-backed [`JobStore`].
#[derive(Clone)]
pub struct PostgresJobStore {
    pool: PgPool,
}

impl PostgresJobStore {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

/// Build a context-rich error for a failed SQL statement.
///
/// Keeps the operation and phase, the tenant/job identifiers, the source error
/// and a recovery next step. Bound payloads (`result_json`, `error`,
/// `progress_json`) are never formatted into the message: they can hold
/// customer data and are not needed to diagnose a database failure.
fn sql_error(
    operation: &str,
    phase: &str,
    tenant: &TenantId,
    id: &str,
    source: &sqlx::Error,
) -> JobError {
    JobError::Internal(format!(
        "{operation} ({phase}): job {id} (tenant {tenant}): {source}; next step: verify \
         Postgres connectivity and the jobs schema, then retry the operation",
        tenant = tenant.as_str()
    ))
}

#[derive(sqlx::FromRow)]
struct JobRow {
    id: String,
    tenant_id: String,
    status: String,
    owner: Option<String>,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
    result_json: Option<String>,
    error: Option<String>,
}

/// `progress_json` is deliberately `None` here: the queries using `JobRow`
/// (`transition`'s `query_as!` and `list`) never select the `progress_json`
/// column, so this conversion cannot report a stale value. `finish`/`fail`
/// must not use it — their terminal `UPDATE ... RETURNING` selects
/// `progress_json` and goes through [`row_to_job_with_progress`], so the
/// response reflects the progress written just before the terminal state.
/// Callers polling progress use [`PostgresJobStore::get`] for the same reason.
fn row_to_job(row: JobRow) -> Job {
    Job {
        id: row.id,
        status: row.status.parse().unwrap_or(JobStatus::Queued),
        owner: row.owner,
        tenant_id: row.tenant_id,
        created_at: row.created_at.to_rfc3339(),
        updated_at: row.updated_at.to_rfc3339(),
        result_json: row.result_json,
        error: row.error,
        progress_json: None,
    }
}

/// Row shape for [`PostgresJobStore::update_progress`], [`PostgresJobStore::get`]
/// and the terminal `finish`/`fail` `UPDATE ... RETURNING`, selecting
/// `progress_json` alongside the columns `query_as!`'s `JobRow` selects
/// elsewhere in this file.
///
/// A runtime `sqlx::query_as` (not the compile-time `query_as!` macro) on
/// purpose: this is the crate's only query touching `progress_json`
/// (added by migration `0003_job_progress.sql`), and regenerating the
/// committed `.sqlx/` offline-query cache requires a live Postgres, which is
/// not assumed to be available at every build. See `hacienda-rag`'s
/// `PgVectorStore` for the same runtime-query convention.
#[derive(sqlx::FromRow)]
struct JobRowWithProgress {
    id: String,
    tenant_id: String,
    status: String,
    owner: Option<String>,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
    result_json: Option<String>,
    error: Option<String>,
    progress_json: Option<String>,
}

fn row_to_job_with_progress(row: JobRowWithProgress) -> Job {
    Job {
        id: row.id,
        status: row.status.parse().unwrap_or(JobStatus::Queued),
        owner: row.owner,
        tenant_id: row.tenant_id,
        created_at: row.created_at.to_rfc3339(),
        updated_at: row.updated_at.to_rfc3339(),
        result_json: row.result_json,
        error: row.error,
        progress_json: row.progress_json,
    }
}

#[async_trait]
impl JobStore for PostgresJobStore {
    async fn create(&self, tenant: &TenantId, owner: Option<String>) -> Result<Job, JobError> {
        let id = Uuid::new_v4().to_string();
        let now = Utc::now();
        let tenant_id = tenant.as_str();

        sqlx::query!(
            r#"
            INSERT INTO jobs (id, tenant_id, status, owner, created_at, updated_at)
            VALUES ($1, $2, $3, $4, $5, $6)
            "#,
            id,
            tenant_id,
            JobStatus::Queued.to_string(),
            owner,
            now,
            now
        )
        .execute(&self.pool)
        .await
        .map_err(|e| sql_error("create", "INSERT", tenant, &id, &e))?;

        Ok(Job {
            id,
            status: JobStatus::Queued,
            owner,
            tenant_id: tenant_id.to_owned(),
            created_at: now.to_rfc3339(),
            updated_at: now.to_rfc3339(),
            result_json: None,
            error: None,
            progress_json: None,
        })
    }

    async fn get(&self, tenant: &TenantId, id: &str) -> Result<Option<Job>, JobError> {
        // Runtime query (not `query_as!`): includes `progress_json` so a
        // poller sees migrate-embeddings-style progress, without requiring a
        // live-Postgres `.sqlx/` cache regeneration for this one query. See
        // `JobRowWithProgress`'s doc comment.
        let row: Option<JobRowWithProgress> = sqlx::query_as(
            "SELECT id, tenant_id, status, owner, created_at, updated_at, result_json, error, progress_json FROM jobs WHERE id = $1 AND tenant_id = $2",
        )
        .bind(id)
        .bind(tenant.as_str())
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| sql_error("get", "SELECT read-back", tenant, id, &e))?;

        Ok(row.map(row_to_job_with_progress))
    }

    async fn update_progress(
        &self,
        tenant: &TenantId,
        id: &str,
        progress_json: String,
    ) -> Result<Job, JobError> {
        let now = Utc::now();

        // Restrict to `running` so a late progress tick cannot overwrite a
        // terminal job's `updated_at` (its retention timestamp). The CAS-style
        // status predicate avoids a read-then-write race with `finish`/`fail`.
        let maybe_row: Option<JobRowWithProgress> = sqlx::query_as(
            r#"
            UPDATE jobs
            SET progress_json = $1, updated_at = $2
            WHERE id = $3 AND tenant_id = $4 AND status = 'running'
            RETURNING id, tenant_id, status, owner, created_at, updated_at, result_json, error, progress_json
            "#,
        )
        .bind(progress_json)
        .bind(now)
        .bind(id)
        .bind(tenant.as_str())
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| sql_error("update_progress", "progress UPDATE", tenant, id, &e))?;

        match maybe_row {
            Some(row) => Ok(row_to_job_with_progress(row)),
            None => {
                // No row means either "not running" (ignore the tick, return
                // the current job) or "no such job" (still NotFound).
                self.get(tenant, id)
                    .await?
                    .ok_or_else(|| JobError::NotFound(id.to_owned()))
            }
        }
    }

    async fn transition(
        &self,
        tenant: &TenantId,
        id: &str,
        from: JobStatus,
        to: JobStatus,
    ) -> Result<Job, JobError> {
        let now = Utc::now();

        // The compare-and-swap stays one atomic statement. When it matches no row,
        // a cheap follow-up read distinguishes "wrong status" from "missing id /
        // wrong tenant" so a cross-tenant id cannot be reported as a
        // `StatusMismatch` (which would confirm the id exists elsewhere). The
        // follow-up read never changes state, so it does not weaken the CAS.
        let maybe_row = sqlx::query_as!(
            JobRow,
            r#"
            UPDATE jobs
            SET status = $1, updated_at = $2
            WHERE id = $3 AND tenant_id = $4 AND status = $5
            RETURNING id, tenant_id, status, owner, created_at, updated_at, result_json, error
            "#,
            to.to_string(),
            now,
            id,
            tenant.as_str(),
            from.to_string()
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| sql_error("transition", "CAS UPDATE", tenant, id, &e))?;

        let Some(row) = maybe_row else {
            // Observe the real current status instead of fabricating one. This
            // read is a best-effort snapshot *after* the losing CAS, so the
            // status may already have moved on; it is still the value actually
            // seen, not a guessed `Queued`. An unrecognised stored value is an
            // internal error rather than a made-up status.
            let observed: Option<String> =
                sqlx::query_scalar("SELECT status FROM jobs WHERE id = $1 AND tenant_id = $2")
                    .bind(id)
                    .bind(tenant.as_str())
                    .fetch_optional(&self.pool)
                    .await
                    .map_err(|e| sql_error("transition", "status read-back", tenant, id, &e))?;
            return Err(match observed {
                None => JobError::NotFound(id.to_owned()),
                Some(raw) => match raw.parse::<JobStatus>() {
                    Ok(actual) => JobError::StatusMismatch {
                        id: id.to_owned(),
                        expected: from,
                        actual,
                    },
                    Err(_) => {
                        JobError::Internal(format!("job {id}: unrecognised stored status '{raw}'"))
                    }
                },
            });
        };

        Ok(row_to_job(row))
    }

    async fn finish(
        &self,
        tenant: &TenantId,
        id: &str,
        result_json: String,
    ) -> Result<Job, JobError> {
        // Runtime query: the terminal-state predicate is new and regenerating
        // the committed `.sqlx/` cache needs a live database, which CI does not
        // assume. The predicate makes a duplicate finish a no-op so the
        // terminal `updated_at` (retention clock) is never refreshed. The
        // server clock stamps that timestamp so a skewed client clock cannot
        // shorten or extend a job's retention window, and `RETURNING
        // progress_json` keeps the last progress tick visible in the terminal
        // response instead of reporting a stale `None`.
        let maybe_row: Option<JobRowWithProgress> = sqlx::query_as(
            r#"
            UPDATE jobs
            SET status = $1, result_json = $2, updated_at = clock_timestamp()
            WHERE id = $3 AND tenant_id = $4 AND status NOT IN ('succeeded','failed')
            RETURNING id, tenant_id, status, owner, created_at, updated_at, result_json, error, progress_json
            "#,
        )
        .bind(JobStatus::Succeeded.to_string())
        .bind(result_json)
        .bind(id)
        .bind(tenant.as_str())
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| sql_error("finish", "terminal UPDATE", tenant, id, &e))?;

        match maybe_row {
            Some(row) => Ok(row_to_job_with_progress(row)),
            None => self
                .get(tenant, id)
                .await?
                .ok_or_else(|| JobError::NotFound(id.to_owned())),
        }
    }

    async fn fail(&self, tenant: &TenantId, id: &str, error: String) -> Result<Job, JobError> {
        // Server clock for the terminal timestamp and `RETURNING
        // progress_json` for the response, for the same reasons as `finish`.
        let maybe_row: Option<JobRowWithProgress> = sqlx::query_as(
            r#"
            UPDATE jobs
            SET status = $1, error = $2, updated_at = clock_timestamp()
            WHERE id = $3 AND tenant_id = $4 AND status NOT IN ('succeeded','failed')
            RETURNING id, tenant_id, status, owner, created_at, updated_at, result_json, error, progress_json
            "#,
        )
        .bind(JobStatus::Failed.to_string())
        .bind(error)
        .bind(id)
        .bind(tenant.as_str())
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| sql_error("fail", "terminal UPDATE", tenant, id, &e))?;

        match maybe_row {
            Some(row) => Ok(row_to_job_with_progress(row)),
            None => self
                .get(tenant, id)
                .await?
                .ok_or_else(|| JobError::NotFound(id.to_owned())),
        }
    }

    async fn list(
        &self,
        tenant: &TenantId,
        filter: Option<JobStatus>,
    ) -> Result<Vec<Job>, JobError> {
        let tenant_id = tenant.as_str();
        let rows = match filter {
            Some(status) => {
                sqlx::query_as!(
                    JobRow,
                    r#"
                    SELECT id, tenant_id, status, owner, created_at, updated_at, result_json, error
                    FROM jobs
                    WHERE tenant_id = $1 AND status = $2
                    ORDER BY created_at, id
                    "#,
                    tenant_id,
                    status.to_string()
                )
                .fetch_all(&self.pool)
                .await
            }
            None => {
                sqlx::query_as!(
                    JobRow,
                    r#"
                    SELECT id, tenant_id, status, owner, created_at, updated_at, result_json, error
                    FROM jobs
                    WHERE tenant_id = $1
                    ORDER BY created_at, id
                    "#,
                    tenant_id
                )
                .fetch_all(&self.pool)
                .await
            }
        }
        // `list` is tenant-wide, so there is no single job id to report; the
        // sentinel keeps the error's job context explicit rather than blank.
        .map_err(|e| sql_error("list", "SELECT for tenant", tenant, "<all>", &e))?;

        Ok(rows.into_iter().map(row_to_job).collect())
    }

    async fn delete(&self, tenant: &TenantId, id: &str) -> Result<bool, JobError> {
        // Runtime query, like `get`/`update_progress`: no new compile-time
        // `.sqlx/` cache entry is needed for this one statement. The tenant
        // predicate makes a cross-tenant delete a silent no-op.
        let rows = sqlx::query("DELETE FROM jobs WHERE id = $1 AND tenant_id = $2")
            .bind(id)
            .bind(tenant.as_str())
            .execute(&self.pool)
            .await
            .map_err(|e| sql_error("delete", "DELETE", tenant, id, &e))?
            .rows_affected();
        Ok(rows > 0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::postgres::test_support;
    use std::sync::Arc;

    // Ignored by default — shares one Postgres instance with the other postgres-feature
    // test modules (see `test_support::shared`), so needs `--test-threads=1`. Run with:
    //   cargo test -p hacienda-core --features postgres \
    //     --lib store::postgres::jobs -- --ignored --test-threads=1

    async fn test_store() -> PostgresJobStore {
        let pool = test_support::shared().await.pool();
        // `fk_jobs_tenant` (migration 0006) requires the tenant to already exist —
        // only `default` is seeded by migration, so this module's shared literal
        // tenant must be admitted before any test's first insert.
        test_support::ensure_tenant(&pool, &t()).await;
        PostgresJobStore::new(pool)
    }

    /// The tenant every test in this module uses. Mirrors `postgres::review::tests::t`:
    /// these tests already share one un-torn-down Postgres database, so a fixed tenant
    /// id keeps that existing shared behaviour rather than accidentally isolating them.
    fn t() -> TenantId {
        TenantId::new("pg-jobs-test-tenant")
    }

    /// The contextual SQL-error helper must carry operation, phase, tenant/job
    /// identifiers, the source error and a next step. No payload argument
    /// exists on the helper, so customer data cannot be formatted in.
    #[test]
    fn sql_error_carries_context_without_payloads() {
        let source = sqlx::Error::InvalidArgument("connection refused".into());
        let err = sql_error("list", "SELECT for tenant", &t(), "<all>", &source);
        let message = err.to_string();
        assert!(message.contains("list"), "operation missing: {message}");
        assert!(
            message.contains("SELECT for tenant"),
            "phase missing: {message}"
        );
        assert!(message.contains(t().as_str()), "tenant missing: {message}");
        assert!(message.contains("<all>"), "job context missing: {message}");
        assert!(
            message.contains("connection refused"),
            "source missing: {message}"
        );
        assert!(message.contains("next step"), "recovery missing: {message}");
    }

    #[test]
    #[ignore]
    fn should_create_and_get_round_trip() {
        test_support::block_on_shared(async {
            let store = test_store().await;

            let job = store
                .create(&t(), Some("tenant-a".to_owned()))
                .await
                .expect("create failed");
            assert_eq!(job.status, JobStatus::Queued);
            assert_eq!(job.owner.as_deref(), Some("tenant-a"));

            let fetched = store
                .get(&t(), &job.id)
                .await
                .expect("get failed")
                .expect("job must exist");
            assert_eq!(fetched.id, job.id);
            assert_eq!(fetched.status, JobStatus::Queued);
        });
    }

    #[test]
    #[ignore]
    fn should_let_exactly_one_concurrent_claim_win() {
        test_support::block_on_shared(async {
            let store = Arc::new(test_store().await);
            let job = store.create(&t(), None).await.expect("create failed");

            let mut handles = Vec::new();
            for _ in 0..8 {
                let store = Arc::clone(&store);
                let id = job.id.clone();
                handles.push(tokio::spawn(async move {
                    store
                        .transition(&t(), &id, JobStatus::Queued, JobStatus::Running)
                        .await
                }));
            }

            let mut wins = 0;
            let mut losses = 0;
            for handle in handles {
                match handle.await.expect("task panicked") {
                    Ok(_) => wins += 1,
                    Err(JobError::StatusMismatch { .. }) => losses += 1,
                    Err(other) => panic!("unexpected error: {other}"),
                }
            }

            assert_eq!(wins, 1, "exactly one concurrent claim must win");
            assert_eq!(losses, 7);

            let final_job = store
                .get(&t(), &job.id)
                .await
                .expect("get failed")
                .expect("job must exist");
            assert_eq!(final_job.status, JobStatus::Running);
        });
    }

    fn t2() -> TenantId {
        TenantId::new("pg-jobs-test-tenant-2")
    }

    /// Durability / reopen: a fresh store instance over the same pool sees the
    /// job written by an earlier instance. This is the Postgres equivalent of
    /// restarting the API process — the state lives in the repository, not the
    /// `Arc` that happened to hold the store.
    #[test]
    #[ignore]
    fn should_reopen_jobs_written_by_a_previous_store_instance() {
        test_support::block_on_shared(async {
            let pool = test_support::shared().await.pool();
            test_support::ensure_tenant(&pool, &t()).await;

            let id = {
                let first = PostgresJobStore::new(pool.clone());
                first
                    .create(&t(), Some("alice".to_owned()))
                    .await
                    .expect("create failed")
                    .id
            };

            let second = PostgresJobStore::new(pool);
            let reopened = second
                .get(&t(), &id)
                .await
                .expect("get failed")
                .expect("job must survive a store reopen");
            assert_eq!(reopened.status, JobStatus::Queued);
            assert_eq!(reopened.owner.as_deref(), Some("alice"));
            assert_eq!(reopened.tenant_id, t().as_str());
        });
    }

    /// A cross-tenant id must not be distinguishable from a missing one: the
    /// CAS reports `NotFound`, never a status-shaped error.
    #[test]
    #[ignore]
    fn cross_tenant_transition_reports_not_found() {
        test_support::block_on_shared(async {
            let pool = test_support::shared().await.pool();
            test_support::ensure_tenant(&pool, &t()).await;
            test_support::ensure_tenant(&pool, &t2()).await;
            let store = PostgresJobStore::new(pool);
            let job = store.create(&t(), None).await.expect("create failed");

            let err = store
                .transition(&t2(), &job.id, JobStatus::Queued, JobStatus::Running)
                .await
                .expect_err("cross-tenant claim must fail");
            assert!(matches!(err, JobError::NotFound(id) if id == job.id));

            let untouched = store
                .get(&t(), &job.id)
                .await
                .expect("get failed")
                .expect("job must exist");
            assert_eq!(untouched.status, JobStatus::Queued);
        });
    }

    /// Retention's delete seam is tenant-scoped even at the repository layer.
    #[test]
    #[ignore]
    fn delete_is_tenant_scoped() {
        test_support::block_on_shared(async {
            let pool = test_support::shared().await.pool();
            test_support::ensure_tenant(&pool, &t()).await;
            test_support::ensure_tenant(&pool, &t2()).await;
            let store = PostgresJobStore::new(pool);
            let job = store.create(&t(), None).await.expect("create failed");

            assert!(!store.delete(&t2(), &job.id).await.expect("delete failed"));
            assert!(store
                .get(&t(), &job.id)
                .await
                .expect("get failed")
                .is_some());
            assert!(store.delete(&t(), &job.id).await.expect("delete failed"));
            assert!(store
                .get(&t(), &job.id)
                .await
                .expect("get failed")
                .is_none());
        });
    }

    /// A tick racing `finish` must not move the terminal timestamp used by
    /// retention, and must not turn a late tick into an error.
    #[test]
    #[ignore]
    fn late_progress_after_finish_is_ignored() {
        test_support::block_on_shared(async {
            let pool = test_support::shared().await.pool();
            test_support::ensure_tenant(&pool, &t()).await;
            let store = PostgresJobStore::new(pool);
            let job = store.create(&t(), None).await.expect("create failed");
            store
                .transition(&t(), &job.id, JobStatus::Queued, JobStatus::Running)
                .await
                .expect("claim failed");
            let finished = store
                .finish(&t(), &job.id, "{}".to_owned())
                .await
                .expect("finish failed");
            let terminal_time = finished.updated_at.clone();

            let after = store
                .update_progress(&t(), &job.id, r#"{"late":true}"#.to_owned())
                .await
                .expect("a late tick must not fail");
            assert_eq!(after.status, JobStatus::Succeeded);
            assert_eq!(after.updated_at, terminal_time);
            assert_eq!(after.progress_json, finished.progress_json);
        });
    }

    /// The terminal-state predicate makes a repeated finish a no-op in the
    /// repository too, not just in the API decorator.
    #[test]
    #[ignore]
    fn duplicate_finish_does_not_refresh_terminal_time() {
        test_support::block_on_shared(async {
            let pool = test_support::shared().await.pool();
            test_support::ensure_tenant(&pool, &t()).await;
            let store = PostgresJobStore::new(pool);
            let job = store.create(&t(), None).await.expect("create failed");
            let first = store
                .finish(&t(), &job.id, r#"{"first":true}"#.to_owned())
                .await
                .expect("finish failed");
            let again = store
                .finish(&t(), &job.id, r#"{"second":true}"#.to_owned())
                .await
                .expect("duplicate finish must not fail");
            assert_eq!(again.updated_at, first.updated_at);
            assert_eq!(again.result_json, first.result_json);
        });
    }

    /// First and duplicate terminal writes must both return the progress
    /// payload written while the job was `Running`, alongside the original
    /// result/reason and status. The terminal `UPDATE ... RETURNING` selects
    /// `progress_json` so the first response is not a stale `None`, and the
    /// duplicate falls back to `get` which already reads the column.
    #[test]
    #[ignore]
    fn terminal_writes_keep_progress_json_and_result() {
        test_support::block_on_shared(async {
            let pool = test_support::shared().await.pool();
            test_support::ensure_tenant(&pool, &t()).await;
            let store = PostgresJobStore::new(pool);
            let progress = r#"{"page":3,"chunks":9}"#.to_owned();

            let succeeded = store.create(&t(), None).await.expect("create failed");
            store
                .transition(&t(), &succeeded.id, JobStatus::Queued, JobStatus::Running)
                .await
                .expect("claim failed");
            store
                .update_progress(&t(), &succeeded.id, progress.clone())
                .await
                .expect("progress failed");
            let first = store
                .finish(&t(), &succeeded.id, r#"{"ok":true}"#.to_owned())
                .await
                .expect("finish failed");
            assert_eq!(first.status, JobStatus::Succeeded);
            assert_eq!(first.result_json.as_deref(), Some(r#"{"ok":true}"#));
            assert_eq!(first.progress_json.as_deref(), Some(progress.as_str()));

            let again = store
                .finish(&t(), &succeeded.id, r#"{"ok":false}"#.to_owned())
                .await
                .expect("duplicate finish must not fail");
            assert_eq!(again.status, JobStatus::Succeeded);
            assert_eq!(again.result_json.as_deref(), Some(r#"{"ok":true}"#));
            assert_eq!(again.progress_json.as_deref(), Some(progress.as_str()));

            let failed = store.create(&t(), None).await.expect("create failed");
            store
                .transition(&t(), &failed.id, JobStatus::Queued, JobStatus::Running)
                .await
                .expect("claim failed");
            store
                .update_progress(&t(), &failed.id, progress.clone())
                .await
                .expect("progress failed");
            let first_fail = store
                .fail(&t(), &failed.id, "first".to_owned())
                .await
                .expect("fail failed");
            assert_eq!(first_fail.status, JobStatus::Failed);
            assert_eq!(first_fail.error.as_deref(), Some("first"));
            assert_eq!(first_fail.progress_json.as_deref(), Some(progress.as_str()));

            let again = store
                .fail(&t(), &failed.id, "second".to_owned())
                .await
                .expect("duplicate fail must not fail");
            assert_eq!(again.status, JobStatus::Failed);
            assert_eq!(again.error.as_deref(), Some("first"));
            assert_eq!(again.progress_json.as_deref(), Some(progress.as_str()));
        });
    }
}
