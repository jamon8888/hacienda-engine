//! Postgres [`JobStore`] implementation.

use crate::jobs::{
    error::JobError,
    types::{Job, JobStatus},
    JobStore,
};
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
    status: String,
    owner: Option<String>,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
    result_json: Option<String>,
    error: Option<String>,
}

fn row_to_job(row: JobRow) -> Job {
    Job {
        id: row.id,
        status: row.status.parse().unwrap_or(JobStatus::Queued),
        owner: row.owner,
        created_at: row.created_at.to_rfc3339(),
        updated_at: row.updated_at.to_rfc3339(),
        result_json: row.result_json,
        error: row.error,
    }
}

#[async_trait]
impl JobStore for PostgresJobStore {
    async fn create(&self, owner: Option<String>) -> Result<Job, JobError> {
        let id = Uuid::new_v4().to_string();
        let now = Utc::now();

        sqlx::query!(
            r#"
            INSERT INTO jobs (id, status, owner, created_at, updated_at)
            VALUES ($1, $2, $3, $4, $5)
            "#,
            id,
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
            created_at: now.to_rfc3339(),
            updated_at: now.to_rfc3339(),
            result_json: None,
            error: None,
        })
    }

    async fn get(&self, id: &str) -> Result<Option<Job>, JobError> {
        let row = sqlx::query_as!(
            JobRow,
            "SELECT id, status, owner, created_at, updated_at, result_json, error FROM jobs WHERE id = $1",
            id
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| JobError::Internal(e.to_string()))?;

        Ok(row.map(row_to_job))
    }

    async fn transition(&self, id: &str, from: JobStatus, to: JobStatus) -> Result<Job, JobError> {
        let now = Utc::now();

        let row = sqlx::query_as!(
            JobRow,
            r#"
            UPDATE jobs
            SET status = $1, updated_at = $2
            WHERE id = $3 AND status = $4
            RETURNING id, status, owner, created_at, updated_at, result_json, error
            "#,
            to.to_string(),
            now,
            id,
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

    async fn finish(&self, id: &str, result_json: String) -> Result<Job, JobError> {
        let now = Utc::now();

        let row = sqlx::query_as!(
            JobRow,
            r#"
            UPDATE jobs
            SET status = $1, result_json = $2, updated_at = $3
            WHERE id = $4
            RETURNING id, status, owner, created_at, updated_at, result_json, error
            "#,
            JobStatus::Succeeded.to_string(),
            result_json,
            now,
            id
        )
        .fetch_one(&self.pool)
        .await
        .map_err(|e| JobError::Internal(e.to_string()))?;

        Ok(row_to_job(row))
    }

    async fn fail(&self, id: &str, error: String) -> Result<Job, JobError> {
        let now = Utc::now();

        let row = sqlx::query_as!(
            JobRow,
            r#"
            UPDATE jobs
            SET status = $1, error = $2, updated_at = $3
            WHERE id = $4
            RETURNING id, status, owner, created_at, updated_at, result_json, error
            "#,
            JobStatus::Failed.to_string(),
            error,
            now,
            id
        )
        .fetch_one(&self.pool)
        .await
        .map_err(|e| JobError::Internal(e.to_string()))?;

        Ok(row_to_job(row))
    }

    async fn list(&self, filter: Option<JobStatus>) -> Result<Vec<Job>, JobError> {
        let rows = match filter {
            Some(status) => {
                sqlx::query_as!(
                    JobRow,
                    r#"
                    SELECT id, status, owner, created_at, updated_at, result_json, error
                    FROM jobs
                    WHERE status = $1
                    ORDER BY created_at, id
                    "#,
                    status.to_string()
                )
                .fetch_all(&self.pool)
                .await
            }
            None => {
                sqlx::query_as!(
                    JobRow,
                    r#"
                    SELECT id, status, owner, created_at, updated_at, result_json, error
                    FROM jobs
                    ORDER BY created_at, id
                    "#
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

    #[tokio::test]
    #[ignore]
    async fn should_create_and_get_round_trip() {
        let store = test_store().await;

        let job = store
            .create(Some("tenant-a".to_owned()))
            .await
            .expect("create failed");
        assert_eq!(job.status, JobStatus::Queued);
        assert_eq!(job.owner.as_deref(), Some("tenant-a"));

        let fetched = store
            .get(&job.id)
            .await
            .expect("get failed")
            .expect("job must exist");
        assert_eq!(fetched.id, job.id);
        assert_eq!(fetched.status, JobStatus::Queued);
    }

    #[tokio::test]
    #[ignore]
    async fn should_let_exactly_one_concurrent_claim_win() {
        let store = Arc::new(test_store().await);
        let job = store.create(None).await.expect("create failed");

        let mut handles = Vec::new();
        for _ in 0..8 {
            let store = Arc::clone(&store);
            let id = job.id.clone();
            handles.push(tokio::spawn(async move {
                store
                    .transition(&id, JobStatus::Queued, JobStatus::Running)
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
            .get(&job.id)
            .await
            .expect("get failed")
            .expect("job must exist");
        assert_eq!(final_job.status, JobStatus::Running);
    }
}
