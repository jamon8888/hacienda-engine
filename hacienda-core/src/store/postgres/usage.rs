//! Postgres usage/metering read-model over the audit chain.
//!
//! Decision 3 (platform-parity spec, §10): usage is derived from the audit chain, not
//! tracked separately — every billable operation already produces an [`AuditEntry`],
//! so usage aggregation is a read-model over `audit_entries`, not a second source of
//! truth that can drift from it.
//!
//! This deliberately queries the table directly rather than going through
//! [`AuditStore::entries`](crate::audit::AuditStore::entries): that method is scoped to
//! the currently open segment (see its doc comment), but a usage read-model needs
//! sealed history too — a billing window that only ever saw the open segment would
//! under-report every time a segment rotates. Postgres holds every entry, sealed or
//! not, in the same table, so this is a plain aggregate query with no segment filter.
//!
//! Billable units actually available on [`AuditEntry`]: entity count (one row per
//! redaction) and `span_length` (bytes of the redacted span), both attributable to a
//! `principal` and windowed by `created_at`. Document count is deliberately **not**
//! reported — entries carry no `document_id`, so it cannot be derived without either a
//! schema change or silently mis-counting documents that produced zero redactions.
//! This resolves the spec's §11 open risk: entries do carry a billable unit, just not
//! every unit Decision 3's framing assumed.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sqlx::PgPool;

/// Error type for usage read-model queries.
#[derive(Debug, thiserror::Error)]
pub enum UsageError {
    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),
}

/// One principal's aggregate usage within the queried window.
#[derive(Debug, Clone, PartialEq)]
pub struct UsageRecord {
    /// `None` groups every entry recorded by an in-process caller
    /// ([`Caller::Trusted`](crate::auth::Caller::Trusted)), matching how
    /// [`AuditEntry::principal`](crate::audit::AuditEntry::principal) itself represents
    /// unattributed entries.
    pub principal: Option<String>,
    pub entity_count: i64,
    pub byte_count: i64,
}

/// Read-model over the audit chain for billing/metering.
#[async_trait]
pub trait UsageStore: Send + Sync {
    /// Aggregate usage per principal, optionally windowed to `created_at >= since` and
    /// `created_at < until`. `None` on either bound leaves that side open.
    async fn summary(
        &self,
        since: Option<DateTime<Utc>>,
        until: Option<DateTime<Utc>>,
    ) -> Result<Vec<UsageRecord>, UsageError>;
}

/// Postgres-backed [`UsageStore`].
#[derive(Clone)]
pub struct PostgresUsageStore {
    pool: PgPool,
}

impl PostgresUsageStore {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

// Row shape for the runtime-checked query below, via `FromRow` rather than the
// `sqlx::query_as!` macro: the macro validates its SQL against either a live database
// or the workspace's committed `.sqlx` offline cache, and this query isn't in that
// cache. `sqlx::query_as` with an explicit `FromRow` type checks column types at
// runtime instead, so it compiles without either — consistent with every other
// Postgres store in this module needing a live `DATABASE_URL` only to *run* its tests,
// never to build.
#[derive(sqlx::FromRow)]
struct UsageRow {
    principal: Option<String>,
    entity_count: i64,
    byte_count: i64,
}

#[async_trait]
impl UsageStore for PostgresUsageStore {
    async fn summary(
        &self,
        since: Option<DateTime<Utc>>,
        until: Option<DateTime<Utc>>,
    ) -> Result<Vec<UsageRecord>, UsageError> {
        let rows: Vec<UsageRow> = sqlx::query_as(
            r#"
            SELECT
                principal,
                COUNT(*) AS entity_count,
                -- `span_length` is BIGINT; `SUM(BIGINT)` in Postgres returns NUMERIC, not
                -- BIGINT, so the runtime type check on `UsageRow::byte_count: i64` fails
                -- without this cast (caught by the ignored integration test below).
                COALESCE(SUM(span_length), 0)::BIGINT AS byte_count
            FROM audit_entries
            WHERE ($1::timestamptz IS NULL OR created_at >= $1)
              AND ($2::timestamptz IS NULL OR created_at < $2)
            GROUP BY principal
            ORDER BY principal NULLS FIRST
            "#,
        )
        .bind(since)
        .bind(until)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows
            .into_iter()
            .map(|row| UsageRecord {
                principal: row.principal,
                entity_count: row.entity_count,
                byte_count: row.byte_count,
            })
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audit::{AuditEntryInput, AuditStore, EntitySource, RedactionAction};
    use crate::store::postgres::audit::PostgresAuditStore;
    use crate::store::postgres::connection::{connect, migrate};
    use uuid::Uuid;

    // Ignored by default — requires a running Postgres. Run with:
    //   DATABASE_URL=postgres://... cargo test -p hacienda-core --features postgres \
    //     --lib store::postgres::usage -- --ignored --test-threads=1

    fn entry(id: &str, principal: Option<&str>, span_length: u32) -> AuditEntryInput {
        AuditEntryInput {
            id: id.to_string(),
            category: "Email".to_string(),
            action: RedactionAction::Mask,
            span_hash: "hash".to_string(),
            span_length,
            confidence: Some(1.0),
            source: EntitySource::Regex,
            pipeline_version: "1.0".to_string(),
            config_hash: "cfg".to_string(),
            principal: principal.map(str::to_string),
        }
    }

    /// Two principals, each with two redactions, must aggregate to distinct rows —
    /// entity count is the row count per principal, byte count is `span_length` summed.
    /// A third, unattributed entry (no token) must group under `principal: None`, not be
    /// dropped or misattributed to either principal.
    #[tokio::test]
    #[ignore]
    async fn should_aggregate_entity_and_byte_counts_per_principal() {
        let database_url =
            std::env::var("DATABASE_URL").expect("DATABASE_URL must be set for integration tests");
        let pool = connect(&database_url).await.expect("connect failed");
        migrate(&pool).await.expect("migrate failed");

        let audit_store = PostgresAuditStore::new(pool.clone());
        let usage_store = PostgresUsageStore::new(pool);

        // `audit_entries` is append-only and never cleaned up between test runs (by
        // design — see the module doc comment), so a fixed principal literal would
        // accumulate rows across repeated invocations and make this test non-idempotent.
        // Suffixing every principal with a fresh UUID keeps each run's rows disjoint
        // from every other run's, past or concurrent.
        let suffix = Uuid::new_v4();
        let avocat_7 = format!("avocat-7-{suffix}");
        let avocat_9 = format!("avocat-9-{suffix}");
        audit_store
            .append(vec![
                entry(&format!("{suffix}-a1"), Some(&avocat_7), 10),
                entry(&format!("{suffix}-a2"), Some(&avocat_7), 15),
                entry(&format!("{suffix}-b1"), Some(&avocat_9), 100),
                entry(&format!("{suffix}-c1"), None, 5),
            ])
            .await
            .expect("append failed");

        let summary = usage_store
            .summary(None, None)
            .await
            .expect("summary failed");

        let avocat_7_record = summary
            .iter()
            .find(|r| r.principal.as_deref() == Some(avocat_7.as_str()))
            .expect("avocat_7 missing from summary");
        assert_eq!(avocat_7_record.entity_count, 2);
        assert_eq!(avocat_7_record.byte_count, 25);

        let avocat_9_record = summary
            .iter()
            .find(|r| r.principal.as_deref() == Some(avocat_9.as_str()))
            .expect("avocat_9 missing from summary");
        assert_eq!(avocat_9_record.entity_count, 1);
        assert_eq!(avocat_9_record.byte_count, 100);

        // Unlike the two principals above, `None` is a shared bucket across every
        // unattributed entry ever inserted (there is no per-run suffix to key on), so
        // this only asserts the just-inserted entry's presence within that bucket's
        // aggregate, not the bucket's exact totals.
        let unattributed = summary
            .iter()
            .find(|r| r.principal.is_none())
            .expect("unattributed entry missing from summary");
        assert!(unattributed.entity_count >= 1);
        assert!(unattributed.byte_count >= 5);
    }

    /// A `since` bound in the future must exclude every entry just inserted — proves the
    /// window filter is applied, not silently ignored.
    #[tokio::test]
    #[ignore]
    async fn since_in_the_future_excludes_everything() {
        let database_url =
            std::env::var("DATABASE_URL").expect("DATABASE_URL must be set for integration tests");
        let pool = connect(&database_url).await.expect("connect failed");
        migrate(&pool).await.expect("migrate failed");

        let audit_store = PostgresAuditStore::new(pool.clone());
        let usage_store = PostgresUsageStore::new(pool);

        let suffix = Uuid::new_v4();
        let principal = format!("avocat-1-{suffix}");
        audit_store
            .append(vec![entry(&format!("{suffix}-x1"), Some(&principal), 10)])
            .await
            .expect("append failed");

        let future = Utc::now() + chrono::Duration::days(1);
        let summary = usage_store
            .summary(Some(future), None)
            .await
            .expect("summary failed");

        assert!(
            summary
                .iter()
                .all(|r| r.principal.as_deref() != Some(principal.as_str())),
            "a since bound in the future must exclude the entry just inserted"
        );
    }
}
