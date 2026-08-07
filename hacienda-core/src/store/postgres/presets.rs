//! Postgres [`PresetStore`] implementation.

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
#[async_trait]
pub trait PresetStore: Send + Sync {
    async fn create(&self, name: &str, config: Value) -> Result<Preset, PresetError>;
    async fn get(&self, id: Uuid) -> Result<Option<Preset>, PresetError>;
    async fn get_by_name(&self, name: &str) -> Result<Option<Preset>, PresetError>;
    async fn list(&self) -> Result<Vec<Preset>, PresetError>;
    async fn delete(&self, id: Uuid) -> Result<(), PresetError>;
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
    async fn create(&self, name: &str, config: Value) -> Result<Preset, PresetError> {
        let row = sqlx::query!(
            "INSERT INTO presets (name, config_json) VALUES ($1, $2) RETURNING id, name, config_json, created_at",
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

    async fn get(&self, id: Uuid) -> Result<Option<Preset>, PresetError> {
        let row = sqlx::query!(
            "SELECT id, name, config_json, created_at FROM presets WHERE id = $1",
            id
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

    async fn get_by_name(&self, name: &str) -> Result<Option<Preset>, PresetError> {
        let row = sqlx::query!(
            "SELECT id, name, config_json, created_at FROM presets WHERE name = $1",
            name
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

    async fn list(&self) -> Result<Vec<Preset>, PresetError> {
        let rows = sqlx::query!(
            "SELECT id, name, config_json, created_at FROM presets ORDER BY created_at"
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

    async fn delete(&self, id: Uuid) -> Result<(), PresetError> {
        sqlx::query!("DELETE FROM presets WHERE id = $1", id)
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

    #[test]
    #[ignore]
    fn should_round_trip_create_get_list_and_delete() {
        test_support::block_on_shared(async {
            let store = test_store().await;
            let name = format!("preset-{}", Uuid::new_v4());
            let config = json!({ "mode": "mask", "categories": ["email", "phone"] });

            let created = store
                .create(&name, config.clone())
                .await
                .expect("create failed");
            assert_eq!(created.name, name);
            assert_eq!(created.config, config);

            let by_id = store
                .get(created.id)
                .await
                .expect("get failed")
                .expect("preset must exist");
            assert_eq!(by_id.id, created.id);

            let by_name = store
                .get_by_name(&name)
                .await
                .expect("get_by_name failed")
                .expect("preset must exist");
            assert_eq!(by_name.id, created.id);

            let all = store.list().await.expect("list failed");
            assert!(all.iter().any(|p| p.id == created.id));

            store.delete(created.id).await.expect("delete failed");
            assert!(store.get(created.id).await.expect("get failed").is_none());
        });
    }
}
