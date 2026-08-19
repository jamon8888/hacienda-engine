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

use crate::tenancy::TenantId;

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
    /// Aggregate usage per principal for `tenant`, optionally windowed to
    /// `created_at >= since` and `created_at < until`. `None` on either bound leaves
    /// that side open.
    ///
    /// Scoped to `tenant` via `audit_entries.segment_id -> audit_segments.tenant_id`
    /// (`audit_entries` itself carries no `tenant_id` column — segments are the unit of
    /// tenant ownership, per S1b). The returned rows carry `principal` values, so an
    /// unscoped aggregate would disclose which principals exist in *other* tenants, not
    /// just leak their usage totals — this join is what closes that gap.
    async fn summary(
        &self,
        tenant: &TenantId,
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
// Postgres store in this module needing a live Postgres only to *run* its tests, never
// to build.
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
        tenant: &TenantId,
        since: Option<DateTime<Utc>>,
        until: Option<DateTime<Utc>>,
    ) -> Result<Vec<UsageRecord>, UsageError> {
        let tenant_id = tenant.as_str();
        let rows: Vec<UsageRow> = sqlx::query_as(
            r#"
            SELECT
                e.principal,
                COUNT(*) AS entity_count,
                -- `span_length` is BIGINT; `SUM(BIGINT)` in Postgres returns NUMERIC, not
                -- BIGINT, so the runtime type check on `UsageRow::byte_count: i64` fails
                -- without this cast (caught by the ignored integration test below).
                COALESCE(SUM(e.span_length), 0)::BIGINT AS byte_count
            FROM audit_entries e
            JOIN audit_segments s ON s.segment_id = e.segment_id
            WHERE s.tenant_id = $1
              AND ($2::timestamptz IS NULL OR e.created_at >= $2)
              AND ($3::timestamptz IS NULL OR e.created_at < $3)
            GROUP BY e.principal
            ORDER BY e.principal NULLS FIRST
            "#,
        )
        .bind(tenant_id)
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
    use crate::store::postgres::test_support;
    use crate::tenancy::TenantId;
    use uuid::Uuid;

    /// The tenant every non-isolation test in this module uses. Mirrors
    /// `postgres::audit::tests::t`: these tests already share one un-torn-down Postgres
    /// database, so a fixed tenant id keeps that existing shared behaviour rather than
    /// accidentally isolating them.
    fn t() -> TenantId {
        TenantId::new("pg-usage-test-tenant")
    }

    // Ignored by default — shares one Postgres instance with the other postgres-feature
    // test modules (see `test_support::shared`), so needs `--test-threads=1`. Run with:
    //   cargo test -p hacienda-core --features postgres \
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
            vertical: None,
        }
    }

    /// Two principals, each with two redactions, must aggregate to distinct rows —
    /// entity count is the row count per principal, byte count is `span_length` summed.
    /// A third, unattributed entry (no token) must group under `principal: None`, not be
    /// dropped or misattributed to either principal.
    #[test]
    #[ignore]
    fn should_aggregate_entity_and_byte_counts_per_principal() {
        test_support::block_on_shared(async {
            let pool = test_support::shared().await.pool();
            // `fk_audit_segments_tenant` (migration 0005) requires the tenant to
            // already exist — only `default` is seeded by migration, so this module's
            // shared literal tenant must be admitted before any test's first append.
            test_support::ensure_tenant(&pool, &t()).await;

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
                .append(
                    &t(),
                    vec![
                        entry(&format!("{suffix}-a1"), Some(&avocat_7), 10),
                        entry(&format!("{suffix}-a2"), Some(&avocat_7), 15),
                        entry(&format!("{suffix}-b1"), Some(&avocat_9), 100),
                        entry(&format!("{suffix}-c1"), None, 5),
                    ],
                )
                .await
                .expect("append failed");

            let summary = usage_store
                .summary(&t(), None, None)
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
        });
    }

    /// A `since` bound in the future must exclude every entry just inserted — proves the
    /// window filter is applied, not silently ignored.
    #[test]
    #[ignore]
    fn since_in_the_future_excludes_everything() {
        test_support::block_on_shared(async {
            let pool = test_support::shared().await.pool();
            test_support::ensure_tenant(&pool, &t()).await;

            let audit_store = PostgresAuditStore::new(pool.clone());
            let usage_store = PostgresUsageStore::new(pool);

            let suffix = Uuid::new_v4();
            let principal = format!("avocat-1-{suffix}");
            audit_store
                .append(
                    &t(),
                    vec![entry(&format!("{suffix}-x1"), Some(&principal), 10)],
                )
                .await
                .expect("append failed");

            let future = Utc::now() + chrono::Duration::days(1);
            let summary = usage_store
                .summary(&t(), Some(future), None)
                .await
                .expect("summary failed");

            assert!(
                summary
                    .iter()
                    .all(|r| r.principal.as_deref() != Some(principal.as_str())),
                "a since bound in the future must exclude the entry just inserted"
            );
        });
    }

    /// Two tenants, each with their own principal, must never see each other's usage —
    /// the exact cross-tenant disclosure CodeRabbit flagged: an unscoped `summary` would
    /// return both principals to either tenant. Each tenant gets a fresh, UUID-suffixed
    /// id so this test cannot inherit contamination from, or contaminate, the rest of
    /// the suite.
    #[test]
    #[ignore]
    fn summary_is_scoped_to_the_requesting_tenant() {
        test_support::block_on_shared(async {
            let pool = test_support::shared().await.pool();
            let tenant_a = TenantId::new(format!("pg-usage-isolation-a-{}", Uuid::new_v4()));
            let tenant_b = TenantId::new(format!("pg-usage-isolation-b-{}", Uuid::new_v4()));
            test_support::ensure_tenant(&pool, &tenant_a).await;
            test_support::ensure_tenant(&pool, &tenant_b).await;

            let audit_store = PostgresAuditStore::new(pool.clone());
            let usage_store = PostgresUsageStore::new(pool);

            let suffix = Uuid::new_v4();
            let principal_a = format!("avocat-a-{suffix}");
            let principal_b = format!("avocat-b-{suffix}");

            audit_store
                .append(
                    &tenant_a,
                    vec![entry(&format!("{suffix}-a1"), Some(&principal_a), 10)],
                )
                .await
                .expect("tenant a append failed");
            audit_store
                .append(
                    &tenant_b,
                    vec![entry(&format!("{suffix}-b1"), Some(&principal_b), 20)],
                )
                .await
                .expect("tenant b append failed");

            let summary_a = usage_store
                .summary(&tenant_a, None, None)
                .await
                .expect("tenant a summary failed");
            assert!(
                summary_a
                    .iter()
                    .any(|r| r.principal.as_deref() == Some(principal_a.as_str())),
                "tenant a's summary must include its own principal"
            );
            assert!(
                summary_a
                    .iter()
                    .all(|r| r.principal.as_deref() != Some(principal_b.as_str())),
                "tenant a's summary must not disclose tenant b's principal"
            );

            let summary_b = usage_store
                .summary(&tenant_b, None, None)
                .await
                .expect("tenant b summary failed");
            assert!(
                summary_b
                    .iter()
                    .any(|r| r.principal.as_deref() == Some(principal_b.as_str())),
                "tenant b's summary must include its own principal"
            );
            assert!(
                summary_b
                    .iter()
                    .all(|r| r.principal.as_deref() != Some(principal_a.as_str())),
                "tenant b's summary must not disclose tenant a's principal"
            );
        });
    }
}
