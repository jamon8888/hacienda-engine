//! Postgres [`JobStore`] implementation.

use crate::jobs::{
    error::JobError,
    types::{Job, JobStatus},
    JobStore,
};
use async_trait::async_trait;
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

#[async_trait]
impl JobStore for PostgresJobStore {
    async fn create(&self, owner: Option<String>) -> Result<Job, JobError> {
        let id = Uuid::new_v4().to_string();
        let now = chrono::Utc::now().to_rfc3339();

        sqlx::query!(
            r#"
            INSERT INTO jobs (id, status, owner, created_at, updated_at)
            VALUES ($1, $2, $3, $4, $5)
            "#,
            id,
            "Queued",
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
            created_at: now.clone(),
            updated_at: now,
            result_json: None,
            error: None,
        })
    }

    async fn get(&self, id: &str) -> Result<Option<Job>, JobError> {
        let row = sqlx::query!(
            "SELECT id, status, owner, created_at, updated_at, result_json, error FROM jobs WHERE id = $1",
            id
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| JobError::Internal(e.to_string()))?;

        Ok(row.map(|row| Job {
            id: row.id,
            status: row.status.parse().unwrap_or(JobStatus::Queued),
            owner: row.owner,
            created_at: row.created_at,
            updated_at: row.updated_at,
            result_json: row.result_json,
            error: row.error,
        }))
    }

    async fn transition(&self, id: &str, from: JobStatus, to: JobStatus) -> Result<Job, JobError> {
        let now = chrono::Utc::now().to_rfc3339();

        let row = sqlx::query!(
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

        Ok(Job {
            id: row.id,
            status: row.status.parse().unwrap_or(JobStatus::Queued),
            owner: row.owner,
            created_at: row.created_at,
            updated_at: row.updated_at,
            result_json: row.result_json,
            error: row.error,
        })
    }

    async fn finish(&self, id: &str, result_json: String) -> Result<Job, JobError> {
        let now = chrono::Utc::now().to_rfc3339();

        let row = sqlx::query!(
            r#"
            UPDATE jobs
            SET status = $1, result_json = $2, updated_at = $3
            WHERE id = $4
            RETURNING id, status, owner, created_at, updated_at, result_json, error
            "#,
            "Succeeded",
            result_json,
            now,
            id
        )
        .fetch_one(&self.pool)
        .await
        .map_err(|e| JobError::Internal(e.to_string()))?;

        Ok(Job {
            id: row.id,
            status: JobStatus::Succeeded,
            owner: row.owner,
            created_at: row.created_at,
            updated_at: row.updated_at,
            result_json: row.result_json,
            error: row.error,
        })
    }

    async fn fail(&self, id: &str, error: String) -> Result<Job, JobError> {
        let now = chrono::Utc::now().to_rfc3339();

        let row = sqlx::query!(
            r#"
            UPDATE jobs
            SET status = $1, error = $2, updated_at = $3
            WHERE id = $4
            RETURNING id, status, owner, created_at, updated_at, result_json, error
            "#,
            "Failed",
            error,
            now,
            id
        )
        .fetch_one(&self.pool)
        .await
        .map_err(|e| JobError::Internal(e.to_string()))?;

        Ok(Job {
            id: row.id,
            status: JobStatus::Failed,
            owner: row.owner,
            created_at: row.created_at,
            updated_at: row.updated_at,
            result_json: row.result_json,
            error: row.error,
        })
    }

    async fn list(&self, filter: Option<JobStatus>) -> Result<Vec<Job>, JobError> {
        let rows = if let Some(status) = filter {
            sqlx::query!(
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
        } else {
            sqlx::query!(
                r#"
                SELECT id, status, owner, created_at, updated_at, result_json, error
                FROM jobs
                ORDER BY created_at, id
                "#
            )
            .fetch_all(&self.pool)
            .await
        }
        .map_err(|e| JobError::Internal(e.to_string()))?;

        Ok(rows
            .into_iter()
            .map(|row| Job {
                id: row.id,
                status: row.status.parse().unwrap_or(JobStatus::Queued),
                owner: row.owner,
                created_at: row.created_at,
                updated_at: row.updated_at,
                result_json: row.result_json,
                error: row.error,
            })
            .collect())
    }
}