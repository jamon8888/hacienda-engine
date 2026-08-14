//! Postgres [`ReviewStore`] implementation.

use crate::review::{
    error::ReviewError,
    types::{Priority, QueueStats, ReviewDecision, ReviewQueueItem, ReviewStatus},
    ReviewStore,
};
use crate::tenancy::TenantId;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sqlx::PgPool;

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
        let deadline = parse_optional_rfc3339(item.deadline.as_deref())?;
        let tenant_id = item.tenant.as_str();

        sqlx::query!(
            r#"
            INSERT INTO review_items (id, tenant_id, text_snippet, category, start_pos, end_pos, confidence, source, status, priority, deadline)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)
            "#,
            item.id,
            tenant_id,
            item.text_snippet,
            item.category,
            item.start as i64,
            item.end as i64,
            item.confidence as f64,
            item.source,
            item.status.to_string(),
            item.priority.to_string(),
            deadline
        )
        .execute(&self.pool)
        .await?;

        Ok(item)
    }

    async fn assign(
        &self,
        tenant: &TenantId,
        id: &str,
        reviewer: &str,
    ) -> Result<ReviewQueueItem, ReviewError> {
        let tenant_id = tenant.as_str();
        let row = sqlx::query_as!(
            ReviewItemRow,
            r#"
            UPDATE review_items
            SET status = 'in_review', assigned_reviewer = $1
            WHERE id = $2 AND tenant_id = $3 AND status = 'pending'
            RETURNING id, text_snippet, category, start_pos, end_pos, confidence, source, status,
                      priority, assigned_reviewer, deadline, created_at, decided_by, decided_at,
                      decision, comment
            "#,
            reviewer,
            id,
            tenant_id
        )
        .fetch_optional(&self.pool)
        .await?;

        match row {
            Some(row) => row_to_item(row, tenant.clone()),
            None => {
                // The item exists (for this tenant) but was not `pending`, or no such
                // item exists for this tenant at all. Distinguish the two so the caller
                // gets an accurate error — `get` already applies the same tenant filter,
                // so a cross-tenant id falls into `NotFound` here exactly like a
                // nonexistent one (D-S1b-1).
                match self.get(tenant, id).await? {
                    Some(existing) => Err(ReviewError::InvalidTransition {
                        from: existing.status.to_string(),
                        to: ReviewStatus::InReview.to_string(),
                    }),
                    None => Err(ReviewError::NotFound(id.to_string())),
                }
            }
        }
    }

    async fn decide(
        &self,
        tenant: &TenantId,
        id: &str,
        decision: ReviewDecision,
        reviewer: &str,
        comment: &str,
    ) -> Result<ReviewQueueItem, ReviewError> {
        let now = Utc::now();
        let status = match decision {
            ReviewDecision::Approve => ReviewStatus::Approved,
            ReviewDecision::Reject => ReviewStatus::Rejected,
            ReviewDecision::Modify => ReviewStatus::Modified,
        };
        let tenant_id = tenant.as_str();

        let row = sqlx::query_as!(
            ReviewItemRow,
            r#"
            UPDATE review_items
            SET status = $1, decided_by = $2, decided_at = $3, decision = $4, comment = $5
            WHERE id = $6 AND tenant_id = $7 AND decision IS NULL
            RETURNING id, text_snippet, category, start_pos, end_pos, confidence, source, status,
                      priority, assigned_reviewer, deadline, created_at, decided_by, decided_at,
                      decision, comment
            "#,
            status.to_string(),
            reviewer,
            now,
            decision.to_string(),
            comment,
            id,
            tenant_id
        )
        .fetch_optional(&self.pool)
        .await?;

        match row {
            Some(row) => row_to_item(row, tenant.clone()),
            None => {
                // Either already decided, or no such item exists for this tenant.
                match self.get(tenant, id).await? {
                    Some(_) => Err(ReviewError::AlreadyDecided(id.to_string())),
                    None => Err(ReviewError::NotFound(id.to_string())),
                }
            }
        }
    }

    async fn list(
        &self,
        tenant: &TenantId,
        filter: Option<ReviewStatus>,
    ) -> Result<Vec<ReviewQueueItem>, ReviewError> {
        let tenant_id = tenant.as_str();
        let items = if let Some(status) = filter {
            let rows = sqlx::query_as!(
                ReviewItemRow,
                r#"
                SELECT id, text_snippet, category, start_pos, end_pos, confidence, source, status,
                       priority, assigned_reviewer, decided_by, decided_at, decision, comment,
                       deadline, created_at
                FROM review_items
                WHERE tenant_id = $1 AND status = $2
                ORDER BY created_at
                "#,
                tenant_id,
                status.to_string()
            )
            .fetch_all(&self.pool)
            .await?;
            rows.into_iter()
                .map(|row| row_to_item(row, tenant.clone()))
                .collect::<Result<Vec<_>, _>>()?
        } else {
            let rows = sqlx::query_as!(
                ReviewItemRow,
                r#"
                SELECT id, text_snippet, category, start_pos, end_pos, confidence, source, status,
                       priority, assigned_reviewer, decided_by, decided_at, decision, comment,
                       deadline, created_at
                FROM review_items
                WHERE tenant_id = $1
                ORDER BY created_at
                "#,
                tenant_id
            )
            .fetch_all(&self.pool)
            .await?;
            rows.into_iter()
                .map(|row| row_to_item(row, tenant.clone()))
                .collect::<Result<Vec<_>, _>>()?
        };

        Ok(items)
    }

    async fn get(
        &self,
        tenant: &TenantId,
        id: &str,
    ) -> Result<Option<ReviewQueueItem>, ReviewError> {
        let tenant_id = tenant.as_str();
        let row = sqlx::query_as!(
            ReviewItemRow,
            r#"
            SELECT id, text_snippet, category, start_pos, end_pos, confidence, source, status,
                   priority, assigned_reviewer, decided_by, decided_at, decision, comment,
                   deadline, created_at
            FROM review_items
            WHERE id = $1 AND tenant_id = $2
            "#,
            id,
            tenant_id
        )
        .fetch_optional(&self.pool)
        .await?;

        row.map(|row| row_to_item(row, tenant.clone())).transpose()
    }

    async fn stats(&self, tenant: &TenantId) -> Result<QueueStats, ReviewError> {
        let tenant_id = tenant.as_str();
        let row = sqlx::query!(
            r#"
            SELECT
                COUNT(*) as total,
                COUNT(*) FILTER (WHERE status = 'pending') as pending,
                COUNT(*) FILTER (WHERE status = 'in_review') as in_review,
                COUNT(*) FILTER (WHERE status = 'approved') as approved,
                COUNT(*) FILTER (WHERE status = 'rejected') as rejected,
                COUNT(*) FILTER (WHERE status = 'modified') as modified
            FROM review_items
            WHERE tenant_id = $1
            "#,
            tenant_id
        )
        .fetch_one(&self.pool)
        .await?;

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

// ── row <-> domain conversion ───────────────────────────────────────────────

struct ReviewItemRow {
    id: String,
    text_snippet: String,
    category: String,
    start_pos: i64,
    end_pos: i64,
    confidence: f64,
    source: String,
    status: String,
    priority: String,
    assigned_reviewer: Option<String>,
    decided_by: Option<String>,
    decided_at: Option<DateTime<Utc>>,
    decision: Option<String>,
    comment: Option<String>,
    deadline: Option<DateTime<Utc>>,
    created_at: DateTime<Utc>,
}

/// `tenant` is passed in rather than selected as a column: every caller already knows it
/// (it came from the `WHERE tenant_id = $n` predicate that produced this row), so
/// selecting it back out would just be a second place for it to disagree with the query.
fn row_to_item(row: ReviewItemRow, tenant: TenantId) -> Result<ReviewQueueItem, ReviewError> {
    Ok(ReviewQueueItem {
        id: row.id,
        tenant,
        text_snippet: row.text_snippet,
        category: row.category,
        start: row.start_pos as u32,
        end: row.end_pos as u32,
        confidence: row.confidence as f32,
        source: row.source,
        status: row
            .status
            .parse::<ReviewStatus>()
            .map_err(ReviewError::Internal)?,
        priority: row
            .priority
            .parse::<Priority>()
            .map_err(ReviewError::Internal)?,
        assigned_reviewer: row.assigned_reviewer,
        created_at: row.created_at.to_rfc3339(),
        deadline: row.deadline.map(|d| d.to_rfc3339()),
        decision: row
            .decision
            .map(|d| d.parse::<ReviewDecision>())
            .transpose()
            .map_err(ReviewError::Internal)?,
        decided_by: row.decided_by,
        decided_at: row.decided_at.map(|d| d.to_rfc3339()),
        comment: row.comment,
    })
}

fn parse_optional_rfc3339(s: Option<&str>) -> Result<Option<DateTime<Utc>>, ReviewError> {
    s.map(|s| {
        DateTime::parse_from_rfc3339(s)
            .map(|d| d.with_timezone(&Utc))
            .map_err(|e| ReviewError::Internal(format!("invalid deadline '{s}': {e}")))
    })
    .transpose()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::postgres::test_support;
    use std::sync::Arc;

    // Ignored by default — shares one Postgres instance with the other postgres-feature
    // test modules (see `test_support::shared`), so needs `--test-threads=1`. Run with:
    //   cargo test -p hacienda-core --features postgres \
    //     --lib store::postgres::review -- --ignored --test-threads=1

    async fn test_store() -> PostgresReviewStore {
        PostgresReviewStore::new(test_support::shared().await.pool())
    }

    /// The tenant every test in this module uses. Mirrors `postgres::audit::tests::t`:
    /// these tests already share one un-torn-down Postgres database, so a fixed tenant
    /// id keeps that existing shared behaviour rather than accidentally isolating them.
    fn t() -> TenantId {
        TenantId::new("pg-review-test-tenant")
    }

    fn test_item(id: &str) -> ReviewQueueItem {
        ReviewQueueItem {
            id: id.to_owned(),
            tenant: t(),
            text_snippet: "jane@example.com".to_owned(),
            category: "email".to_owned(),
            start: 0,
            end: 16,
            confidence: 0.4,
            source: "regex".to_owned(),
            status: ReviewStatus::Pending,
            priority: Priority::High,
            assigned_reviewer: None,
            created_at: Utc::now().to_rfc3339(),
            deadline: None,
            decision: None,
            decided_by: None,
            decided_at: None,
            comment: None,
        }
    }

    #[test]
    #[ignore]
    fn should_submit_and_get_round_trip() {
        test_support::block_on_shared(async {
            let store = test_store().await;
            let id = uuid::Uuid::new_v4().to_string();
            let item = test_item(&id);

            store.submit(item.clone()).await.expect("submit failed");

            let fetched = store
                .get(&t(), &id)
                .await
                .expect("get failed")
                .expect("item must exist");
            assert_eq!(fetched.id, id);
            assert_eq!(fetched.text_snippet, "jane@example.com");
            assert_eq!(fetched.status, ReviewStatus::Pending);
            assert_eq!(fetched.priority, Priority::High);
        });
    }

    #[test]
    #[ignore]
    fn should_let_exactly_one_concurrent_assign_win() {
        test_support::block_on_shared(async {
            let store = Arc::new(test_store().await);
            let id = uuid::Uuid::new_v4().to_string();
            store.submit(test_item(&id)).await.expect("submit failed");

            let mut handles = Vec::new();
            for i in 0..8 {
                let store = Arc::clone(&store);
                let id = id.clone();
                handles.push(tokio::spawn(async move {
                    store.assign(&t(), &id, &format!("reviewer-{i}")).await
                }));
            }

            let mut wins = 0;
            let mut losses = 0;
            for handle in handles {
                match handle.await.expect("task panicked") {
                    Ok(_) => wins += 1,
                    Err(ReviewError::InvalidTransition { .. }) => losses += 1,
                    Err(other) => panic!("unexpected error: {other}"),
                }
            }

            assert_eq!(wins, 1, "exactly one concurrent assign must win");
            assert_eq!(losses, 7);

            let final_item = store
                .get(&t(), &id)
                .await
                .expect("get failed")
                .expect("item must exist");
            assert_eq!(final_item.status, ReviewStatus::InReview);
        });
    }

    #[test]
    #[ignore]
    fn should_let_exactly_one_concurrent_decide_win() {
        test_support::block_on_shared(async {
            let store = Arc::new(test_store().await);
            let id = uuid::Uuid::new_v4().to_string();
            store.submit(test_item(&id)).await.expect("submit failed");
            store
                .assign(&t(), &id, "reviewer-0")
                .await
                .expect("assign failed");

            let mut handles = Vec::new();
            for i in 0..8 {
                let store = Arc::clone(&store);
                let id = id.clone();
                handles.push(tokio::spawn(async move {
                    store
                        .decide(
                            &t(),
                            &id,
                            ReviewDecision::Approve,
                            &format!("reviewer-{i}"),
                            "looks fine",
                        )
                        .await
                }));
            }

            let mut wins = 0;
            let mut losses = 0;
            for handle in handles {
                match handle.await.expect("task panicked") {
                    Ok(_) => wins += 1,
                    Err(ReviewError::AlreadyDecided(_)) => losses += 1,
                    Err(other) => panic!("unexpected error: {other}"),
                }
            }

            assert_eq!(wins, 1, "exactly one concurrent decide must win");
            assert_eq!(losses, 7);

            let final_item = store
                .get(&t(), &id)
                .await
                .expect("get failed")
                .expect("item must exist");
            assert_eq!(final_item.status, ReviewStatus::Approved);
        });
    }
}
