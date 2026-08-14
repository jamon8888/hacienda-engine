//! Postgres [`PresetStore`] implementation.

use crate::tenancy::TenantId;
use async_trait::async_trait;
use serde_json::Value;
use sqlx::PgPool;
use uuid::Uuid;

/// Error type for preset store operations.
#[derive(Debug, thiserror::Error)]
pub enum PresetError {
    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),

    #[error("preset not found")]
    NotFound,
}

/// Trait for preset storage.
///
/// # Tenant scoping (S1b)
///
/// Every method takes `&TenantId`. `name` is caller-supplied, the same
/// caller-supplied-identifier situation as `DocumentVersionStore::create_version`'s
/// `document_id` — two tenants must be free to each name a preset `"default"` without
/// colliding, which the `presets_tenant_name_key` constraint (replacing the old bare
/// `presets_name_key`) now enforces at the schema level, not just in application code.
/// `get`/`get_by_name` on an id/name belonging to a different tenant resolve as `None`,
/// indistinguishable from one that never existed (D-S1b-1).
#[async_trait]
pub trait PresetStore: Send + Sync {
    async fn create(
        &self,
        tenant: &TenantId,
        name: &str,
        config: Value,
    ) -> Result<Preset, PresetError>;
    async fn get(&self, tenant: &TenantId, id: Uuid) -> Result<Option<Preset>, PresetError>;
    async fn get_by_name(
        &self,
        tenant: &TenantId,
        name: &str,
    ) -> Result<Option<Preset>, PresetError>;
    async fn list(&self, tenant: &TenantId) -> Result<Vec<Preset>, PresetError>;
    async fn delete(&self, tenant: &TenantId, id: Uuid) -> Result<(), PresetError>;
}

/// A preset record.
#[derive(Debug, Clone)]
pub struct Preset {
    pub id: Uuid,
    pub name: String,
    pub config: Value,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

/// Postgres-backed [`PresetStore`].
#[derive(Clone)]
pub struct PostgresPresetStore {
    pool: PgPool,
}

impl PostgresPresetStore {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl PresetStore for PostgresPresetStore {
    async fn create(
        &self,
        tenant: &TenantId,
        name: &str,
        config: Value,
    ) -> Result<Preset, PresetError> {
        let row = sqlx::query!(
            "INSERT INTO presets (tenant_id, name, config_json) VALUES ($1, $2, $3) RETURNING id, name, config_json, created_at",
            tenant.as_str(),
            name,
            config
        )
        .fetch_one(&self.pool)
        .await?;

        Ok(Preset {
            id: row.id,
            name: row.name,
            config: row.config_json,
            created_at: row.created_at,
        })
    }

    async fn get(&self, tenant: &TenantId, id: Uuid) -> Result<Option<Preset>, PresetError> {
        let row = sqlx::query!(
            "SELECT id, name, config_json, created_at FROM presets WHERE id = $1 AND tenant_id = $2",
            id,
            tenant.as_str()
        )
        .fetch_optional(&self.pool)
        .await?;

        Ok(row.map(|row| Preset {
            id: row.id,
            name: row.name,
            config: row.config_json,
            created_at: row.created_at,
        }))
    }

    async fn get_by_name(
        &self,
        tenant: &TenantId,
        name: &str,
    ) -> Result<Option<Preset>, PresetError> {
        let row = sqlx::query!(
            "SELECT id, name, config_json, created_at FROM presets WHERE name = $1 AND tenant_id = $2",
            name,
            tenant.as_str()
        )
        .fetch_optional(&self.pool)
        .await?;

        Ok(row.map(|row| Preset {
            id: row.id,
            name: row.name,
            config: row.config_json,
            created_at: row.created_at,
        }))
    }

    async fn list(&self, tenant: &TenantId) -> Result<Vec<Preset>, PresetError> {
        let rows = sqlx::query!(
            "SELECT id, name, config_json, created_at FROM presets WHERE tenant_id = $1 ORDER BY created_at",
            tenant.as_str()
        )
        .fetch_all(&self.pool)
        .await?;

        Ok(rows
            .into_iter()
            .map(|row| Preset {
                id: row.id,
                name: row.name,
                config: row.config_json,
                created_at: row.created_at,
            })
            .collect())
    }

    async fn delete(&self, tenant: &TenantId, id: Uuid) -> Result<(), PresetError> {
        sqlx::query!(
            "DELETE FROM presets WHERE id = $1 AND tenant_id = $2",
            id,
            tenant.as_str()
        )
        .execute(&self.pool)
        .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::postgres::test_support;
    use serde_json::json;

    // Ignored by default — shares one Postgres instance with the other postgres-feature
    // test modules (see `test_support::shared`), so needs `--test-threads=1`. Run with:
    //   cargo test -p hacienda-core --features postgres \
    //     --lib store::postgres::presets -- --ignored --test-threads=1

    async fn test_store() -> PostgresPresetStore {
        PostgresPresetStore::new(test_support::shared().await.pool())
    }

    /// The tenant every test in this module uses. Mirrors `postgres::versions::tests::t`:
    /// these tests already share one un-torn-down Postgres database, so a fixed tenant
    /// id keeps that existing shared behaviour rather than accidentally isolating them.
    fn t() -> TenantId {
        TenantId::new("pg-presets-test-tenant")
    }

    #[test]
    #[ignore]
    fn should_round_trip_create_get_list_and_delete() {
        test_support::block_on_shared(async {
            let store = test_store().await;
            let name = format!("preset-{}", Uuid::new_v4());
            let config = json!({ "mode": "mask", "categories": ["email", "phone"] });

            let created = store
                .create(&t(), &name, config.clone())
                .await
                .expect("create failed");
            assert_eq!(created.name, name);
            assert_eq!(created.config, config);

            let by_id = store
                .get(&t(), created.id)
                .await
                .expect("get failed")
                .expect("preset must exist");
            assert_eq!(by_id.id, created.id);

            let by_name = store
                .get_by_name(&t(), &name)
                .await
                .expect("get_by_name failed")
                .expect("preset must exist");
            assert_eq!(by_name.id, created.id);

            let all = store.list(&t()).await.expect("list failed");
            assert!(all.iter().any(|p| p.id == created.id));

            store.delete(&t(), created.id).await.expect("delete failed");
            assert!(store
                .get(&t(), created.id)
                .await
                .expect("get failed")
                .is_none());
        });
    }

    /// D-S1b-1: an id/name that belongs to a different tenant must be reported exactly
    /// like one that does not exist at all — `None`, never a distinguishable error.
    #[test]
    #[ignore]
    fn preset_get_for_another_tenants_id_or_name_is_not_found() {
        test_support::block_on_shared(async {
            let store = test_store().await;
            let other_tenant = TenantId::new("pg-presets-test-tenant-other");
            let name = format!("preset-{}", Uuid::new_v4());
            let config = json!({ "mode": "mask" });

            let created = store
                .create(&t(), &name, config)
                .await
                .expect("create failed");

            assert!(store.get(&t(), created.id).await.unwrap().is_some());
            assert!(store
                .get(&other_tenant, created.id)
                .await
                .unwrap()
                .is_none());
            assert!(store
                .get_by_name(&other_tenant, &name)
                .await
                .unwrap()
                .is_none());

            let other_tenant_presets = store.list(&other_tenant).await.unwrap();
            assert!(other_tenant_presets.iter().all(|p| p.id != created.id));
        });
    }

    /// The exact functional bug this task set out to fix, per spec §5:
    /// `preset_names_collide_across_tenants_without_error` — two tenants can each create a
    /// preset named `"default"`, which the old bare `UNIQUE(name)` made impossible.
    #[test]
    #[ignore]
    fn preset_names_collide_across_tenants_without_error() {
        test_support::block_on_shared(async {
            let store = test_store().await;
            let other_tenant = TenantId::new("pg-presets-test-tenant-other2");
            // Unique per test run (these tests share one un-torn-down database) but
            // identical across the two tenants within this run — that's the collision
            // being exercised.
            let name = format!("default-{}", Uuid::new_v4());

            let a = store
                .create(&t(), &name, json!({ "mode": "mask" }))
                .await
                .expect("tenant a create must succeed");
            let b = store
                .create(&other_tenant, &name, json!({ "mode": "redact" }))
                .await
                .expect("tenant b create with the same name must also succeed");

            assert_ne!(a.id, b.id);
            assert_eq!(a.name, b.name);

            let a_fetched = store
                .get_by_name(&t(), &name)
                .await
                .unwrap()
                .expect("tenant a's preset");
            assert_eq!(a_fetched.id, a.id);

            let b_fetched = store
                .get_by_name(&other_tenant, &name)
                .await
                .unwrap()
                .expect("tenant b's preset");
            assert_eq!(b_fetched.id, b.id);
        });
    }
}
