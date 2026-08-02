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