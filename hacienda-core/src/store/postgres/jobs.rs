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

/// `progress_json` is always `None` here: `JobRow`'s `query_as!` (used by
/// `transition`/`finish`/`fail`, none of which touch the `progress_json`
/// column) never selects it, so this conversion cannot report a stale value.
/// Callers polling progress must use [`PostgresJobStore::get`], which reads
/// the column via [`row_to_job_with_progress`].
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

/// Row shape for [`PostgresJobStore::update_progress`], selecting
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
        .map_err(|e| JobError::Internal(e.to_string()))?;

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
        .map_err(|e| JobError::Internal(e.to_string()))?;

        Ok(row.map(row_to_job_with_progress))
    }

    async fn update_progress(
        &self,
        tenant: &TenantId,
        id: &str,
        progress_json: String,
    ) -> Result<Job, JobError> {
        let now = Utc::now();

        let row: JobRowWithProgress = sqlx::query_as(
            r#"
            UPDATE jobs
            SET progress_json = $1, updated_at = $2
            WHERE id = $3 AND tenant_id = $4
            RETURNING id, tenant_id, status, owner, created_at, updated_at, result_json, error, progress_json
            "#,
        )
        .bind(progress_json)
        .bind(now)
        .bind(id)
        .bind(tenant.as_str())
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| JobError::Internal(e.to_string()))?
        .ok_or_else(|| JobError::NotFound(id.to_owned()))?;

        Ok(row_to_job_with_progress(row))
    }

    async fn transition(
        &self,
        tenant: &TenantId,
        id: &str,
        from: JobStatus,
        to: JobStatus,
    ) -> Result<Job, JobError> {
        let now = Utc::now();

        // Matching on id, tenant, and status together means a cross-tenant id falls into
        // the same not-found-shaped path a status mismatch does — see this method's
        // pre-existing (not new) limitation: the `RETURNING`-based compare-and-swap
        // cannot distinguish "no such id"/"wrong tenant" from "wrong status" without a
        // second query, so both report `StatusMismatch` with a best-effort `actual`. This
        // predates tenant scoping; the `AND tenant_id = $n` predicate here only closes the
        // isolation gap, it does not change that pre-existing imprecision.
        let row = sqlx::query_as!(
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
        .map_err(|e| JobError::Internal(e.to_string()))?
        .ok_or_else(|| JobError::StatusMismatch {
            id: id.to_owned(),
            expected: from,
            actual: JobStatus::Queued, // We don't know actual from the query
        })?;

        Ok(row_to_job(row))
    }

    async fn finish(
        &self,
        tenant: &TenantId,
        id: &str,
        result_json: String,
    ) -> Result<Job, JobError> {
        let now = Utc::now();

        let row = sqlx::query_as!(
            JobRow,
            r#"
            UPDATE jobs
            SET status = $1, result_json = $2, updated_at = $3
            WHERE id = $4 AND tenant_id = $5
            RETURNING id, tenant_id, status, owner, created_at, updated_at, result_json, error
            "#,
            JobStatus::Succeeded.to_string(),
            result_json,
            now,
            id,
            tenant.as_str()
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| {
            JobError::Internal(format!(
                "finish: job {id} (tenant {tenant}): {e}",
                tenant = tenant.as_str()
            ))
        })?
        .ok_or_else(|| JobError::NotFound(id.to_owned()))?;

        Ok(row_to_job(row))
    }

    async fn fail(&self, tenant: &TenantId, id: &str, error: String) -> Result<Job, JobError> {
        let now = Utc::now();

        let row = sqlx::query_as!(
            JobRow,
            r#"
            UPDATE jobs
            SET status = $1, error = $2, updated_at = $3
            WHERE id = $4 AND tenant_id = $5
            RETURNING id, tenant_id, status, owner, created_at, updated_at, result_json, error
            "#,
            JobStatus::Failed.to_string(),
            error,
            now,
            id,
            tenant.as_str()
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| {
            JobError::Internal(format!(
                "fail: job {id} (tenant {tenant}): {e}",
                tenant = tenant.as_str()
            ))
        })?
        .ok_or_else(|| JobError::NotFound(id.to_owned()))?;

        Ok(row_to_job(row))
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
        .map_err(|e| JobError::Internal(e.to_string()))?;

        Ok(rows.into_iter().map(row_to_job).collect())
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
        PostgresJobStore::new(test_support::shared().await.pool())
    }

    /// The tenant every test in this module uses. Mirrors `postgres::review::tests::t`:
    /// these tests already share one un-torn-down Postgres database, so a fixed tenant
    /// id keeps that existing shared behaviour rather than accidentally isolating them.
    fn t() -> TenantId {
        TenantId::new("pg-jobs-test-tenant")
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
}
