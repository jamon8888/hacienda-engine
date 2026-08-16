//! Postgres [`TenantStore`] implementation.

use crate::tenancy::{store::TenantStore, Tenant, TenantError, TenantId};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sqlx::PgPool;

/// Postgres-backed [`TenantStore`].
#[derive(Clone)]
pub struct PostgresTenantStore {
    pool: PgPool,
}

impl PostgresTenantStore {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

struct TenantRow {
    id: String,
    display_name: Option<String>,
    created_at: DateTime<Utc>,
}

fn row_to_tenant(row: TenantRow) -> Tenant {
    Tenant {
        id: TenantId::new(row.id),
        display_name: row.display_name,
        created_at: row.created_at.to_rfc3339(),
    }
}

#[async_trait]
impl TenantStore for PostgresTenantStore {
    async fn create(
        &self,
        id: TenantId,
        display_name: Option<String>,
    ) -> Result<Tenant, TenantError> {
        let id_str = id.as_str().to_owned();
        // `INSERT ... ON CONFLICT DO NOTHING` plus a row-count check, rather than a
        // plain `INSERT`, so a duplicate id surfaces as the same `AlreadyExists` this
        // trait's in-memory backend returns instead of a raw unique-constraint
        // `sqlx::Error` a caller would have to pattern-match on database-specific text.
        let result = sqlx::query!(
            r#"
            INSERT INTO tenants (id, display_name)
            VALUES ($1, $2)
            ON CONFLICT (id) DO NOTHING
            "#,
            id_str,
            display_name
        )
        .execute(&self.pool)
        .await?;

        if result.rows_affected() == 0 {
            return Err(TenantError::AlreadyExists(id));
        }

        self.get(&id)
            .await?
            .ok_or_else(|| TenantError::Internal("insert succeeded but row not found".into()))
    }

    async fn get(&self, id: &TenantId) -> Result<Option<Tenant>, TenantError> {
        let row = sqlx::query_as!(
            TenantRow,
            r#"SELECT id, display_name, created_at FROM tenants WHERE id = $1"#,
            id.as_str()
        )
        .fetch_optional(&self.pool)
        .await?;

        Ok(row.map(row_to_tenant))
    }

    async fn list(&self) -> Result<Vec<Tenant>, TenantError> {
        let rows = sqlx::query_as!(
            TenantRow,
            r#"SELECT id, display_name, created_at FROM tenants ORDER BY created_at, id"#
        )
        .fetch_all(&self.pool)
        .await?;

        Ok(rows.into_iter().map(row_to_tenant).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::postgres::test_support;

    // Ignored by default — shares one Postgres instance with the other postgres-feature
    // test modules (see `test_support::shared`), so needs `--test-threads=1`. Run with:
    //   cargo test -p hacienda-core --features postgres \
    //     --lib store::postgres::tenants -- --ignored --test-threads=1

    async fn test_store() -> PostgresTenantStore {
        PostgresTenantStore::new(test_support::shared().await.pool())
    }

    #[test]
    #[ignore]
    fn should_create_and_get_round_trip() {
        test_support::block_on_shared(async {
            let store = test_store().await;
            // Unique id per run: the shared fixture's `tenants` table persists across
            // every test in this module (and any other postgres-feature test using the
            // same database), so a fixed id would collide on a second run.
            let id = TenantId::new(format!("tenant-{}", uuid::Uuid::new_v4()));

            let tenant = store
                .create(id.clone(), Some("Acme Corp".to_owned()))
                .await
                .expect("create failed");
            assert_eq!(tenant.id, id);
            assert_eq!(tenant.display_name.as_deref(), Some("Acme Corp"));

            let fetched = store
                .get(&id)
                .await
                .expect("get failed")
                .expect("tenant must exist");
            assert_eq!(fetched, tenant);
        });
    }

    #[test]
    #[ignore]
    fn should_refuse_to_create_the_same_tenant_id_twice() {
        test_support::block_on_shared(async {
            let store = test_store().await;
            let id = TenantId::new(format!("tenant-{}", uuid::Uuid::new_v4()));

            store.create(id.clone(), None).await.expect("first create");
            let err = store
                .create(id.clone(), Some("second".to_owned()))
                .await
                .expect_err("second create must fail");
            assert!(matches!(err, TenantError::AlreadyExists(got) if got == id));
        });
    }
}
