//! Postgres [`ReviewStore`] implementation.

use crate::review::{
    error::ReviewError,
    types::{QueueStats, ReviewDecision, ReviewQueueItem, ReviewStatus},
    ReviewStore,
};
use async_trait::async_trait;
use sqlx::PgPool;
use std::sync::Arc;

/// Postgres-backed [`ReviewStore`].
#[derive(Clone)]
pub struct PostgresReviewStore {
    pool: PgPool,
}

impl PostgresReviewStore {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl ReviewStore for PostgresReviewStore {
    async fn submit(&self, item: ReviewQueueItem) -> Result<ReviewQueueItem, ReviewError> {
        sqlx::query!(
            r#"
            INSERT INTO review_items (id, text_snippet, category, start_pos, end_pos, confidence, source, status, deadline)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
            "#,
            item.id,
            item.text_snippet,
            item.category,
            item.start as i64,
            item.end as i64,
            item.confidence,
            item.source,
            "Pending",
            item.deadline
        )
        .execute(&self.pool)
        .await
        .map_err(|e| ReviewError::Internal(e.to_string()))?;

        Ok(item)
    }

    async fn assign(&self, id: &str, reviewer: &str) -> Result<ReviewQueueItem, ReviewError> {
        let row = sqlx::query!(
            r#"
            UPDATE review_items
            SET status = 'InReview', assigned_reviewer = $1
            WHERE id = $2 AND status = 'Pending'
            RETURNING id, text_snippet, category, start_pos, end_pos, confidence, source, status, assigned_reviewer, deadline, created_at
            "#,
            reviewer,
            id
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| ReviewError::Internal(e.to_string()))?
        .ok_or(ReviewError::NotFound(id.to_string()))?;

        // If status wasn't Pending, the row wouldn't be returned
        if row.status != "InReview" {
            return Err(ReviewError::InvalidTransition {
                from: row.status,
                to: "InReview".to_string(),
            });
        }

        Ok(ReviewQueueItem {
            id: row.id,
            text_snippet: row.text_snippet,
            category: row.category,
            start: row.start_pos as u32,
            end: row.end_pos as u32,
            confidence: row.confidence,
            source: row.source,
            status: ReviewStatus::InReview,
            assigned_reviewer: row.assigned_reviewer,
            decided_by: None,
            decided_at: None,
            decision: None,
            comment: None,
            deadline: row.deadline,
            created_at: row.created_at,
        })
    }

    async fn decide(
        &self,
        id: &str,
        decision: ReviewDecision,
        reviewer: &str,
        comment: &str,
    ) -> Result<ReviewQueueItem, ReviewError> {
        let now = chrono::Utc::now();

        let row = sqlx::query!(
            r#"
            UPDATE review_items
            SET status = $1, decided_by = $2, decided_at = $3, decision = $4, comment = $5
            WHERE id = $6 AND decision IS NULL
            RETURNING id, text_snippet, category, start_pos, end_pos, confidence, source, status, assigned_reviewer, deadline, created_at, decided_by, decided_at, decision, comment
            "#,
            match decision {
                ReviewDecision::Approve => "Approved",
                ReviewDecision::Reject => "Rejected",
                ReviewDecision::Modify => "Modified",
            },
            reviewer,
            now,
            match decision {
                ReviewDecision::Approve => "Approve",
                ReviewDecision::Reject => "Reject",
                ReviewDecision::Modify => "Modify",
            },
            comment,
            id
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| ReviewError::Internal(e.to_string()))?
        .ok_or_else(|| ReviewError::AlreadyDecided(id.to_string()))?;

        Ok(ReviewQueueItem {
            id: row.id,
            text_snippet: row.text_snippet,
            category: row.category,
            start: row.start_pos as u32,
            end: row.end_pos as u32,
            confidence: row.confidence,
            source: row.source,
            status: row.status.parse().unwrap_or(ReviewStatus::Pending),
            assigned_reviewer: row.assigned_reviewer,
            decided_by: row.decided_by,
            decided_at: row.decided_at.map(|d| d.to_rfc3339()),
            decision: row.decision.map(|d| d.parse().unwrap_or(ReviewDecision::Approve)),
            comment: row.comment,
            deadline: row.deadline,
            created_at: row.created_at,
        })
    }

    async fn list(
        &self,
        filter: Option<ReviewStatus>,
    ) -> Result<Vec<ReviewQueueItem>, ReviewError> {
        let rows = if let Some(status) = filter {
            sqlx::query!(
                r#"
                SELECT id, text_snippet, category, start_pos, end_pos, confidence, source, status,
                       assigned_reviewer, decided_by, decided_at, decision, comment, deadline, created_at
                FROM review_items
                WHERE status = $1
                ORDER BY created_at
                "#,
                status.to_string()
            )
            .fetch_all(&self.pool)
            .await
        } else {
            sqlx::query!(
                r#"
                SELECT id, text_snippet, category, start_pos, end_pos, confidence, source, status,
                       assigned_reviewer, decided_by, decided_at, decision, comment, deadline, created_at
                FROM review_items
                ORDER BY created_at
                "#
            )
            .fetch_all(&self.pool)
            .await
        }
        .map_err(|e| ReviewError::Internal(e.to_string()))?;

        Ok(rows
            .into_iter()
            .map(|row| ReviewQueueItem {
                id: row.id,
                text_snippet: row.text_snippet,
                category: row.category,
                start: row.start_pos as u32,
                end: row.end_pos as u32,
                confidence: row.confidence,
                source: row.source,
                status: row.status.parse().unwrap_or(ReviewStatus::Pending),
                assigned_reviewer: row.assigned_reviewer,
                decided_by: row.decided_by,
                decided_at: row.decided_at.map(|d| d.to_rfc3339()),
                decision: row.decision.map(|d| d.parse().unwrap_or(ReviewDecision::Approve)),
                comment: row.comment,
                deadline: row.deadline,
                created_at: row.created_at,
            })
            .collect())
    }

    async fn get(&self, id: &str) -> Result<Option<ReviewQueueItem>, ReviewError> {
        let row = sqlx::query!(
            r#"
            SELECT id, text_snippet, category, start_pos, end_pos, confidence, source, status,
                   assigned_reviewer, decided_by, decided_at, decision, comment, deadline, created_at
            FROM review_items
            WHERE id = $1
            "#,
            id
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| ReviewError::Internal(e.to_string()))?;

        Ok(row.map(|row| ReviewQueueItem {
            id: row.id,
            text_snippet: row.text_snippet,
            category: row.category,
            start: row.start_pos as u32,
            end: row.end_pos as u32,
            confidence: row.confidence,
            source: row.source,
            status: row.status.parse().unwrap_or(ReviewStatus::Pending),
            assigned_reviewer: row.assigned_reviewer,
            decided_by: row.decided_by,
            decided_at: row.decided_at.map(|d| d.to_rfc3339()),
            decision: row.decision.map(|d| d.parse().unwrap_or(ReviewDecision::Approve)),
            comment: row.comment,
            deadline: row.deadline,
            created_at: row.created_at,
        }))
    }

    async fn stats(&self) -> Result<QueueStats, ReviewError> {
        let row = sqlx::query!(
            r#"
            SELECT
                COUNT(*) as total,
                COUNT(*) FILTER (WHERE status = 'Pending') as pending,
                COUNT(*) FILTER (WHERE status = 'InReview') as in_review,
                COUNT(*) FILTER (WHERE status = 'Approved') as approved,
                COUNT(*) FILTER (WHERE status = 'Rejected') as rejected,
                COUNT(*) FILTER (WHERE status = 'Modified') as modified
            FROM review_items
            "#
        )
        .fetch_one(&self.pool)
        .await
        .map_err(|e| ReviewError::Internal(e.to_string()))?;

        Ok(QueueStats {
            total: row.total.unwrap_or(0) as usize,
            pending: row.pending.unwrap_or(0) as usize,
            in_review: row.in_review.unwrap_or(0) as usize,
            approved: row.approved.unwrap_or(0) as usize,
            rejected: row.rejected.unwrap_or(0) as usize,
            modified: row.modified.unwrap_or(0) as usize,
        })
    }

    async fn close(&self) -> Result<(), ReviewError> {
        // No persistent connections to close for sqlx pool
        Ok(())
    }
}