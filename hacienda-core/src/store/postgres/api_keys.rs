//! Postgres [`ApiKeyStore`] implementation.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde_json::Value;
use sqlx::PgPool;
use uuid::Uuid;

/// Error type for API key store operations.
#[derive(Debug, thiserror::Error)]
pub enum ApiKeyError {
    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),

    #[error("API key not found")]
    NotFound,
}

/// Trait for API key storage.
#[async_trait]
pub trait ApiKeyStore: Send + Sync {
    /// Create a new API key record (stores hash only).
    async fn create(&self, key_hash: &str, owner: &str, capabilities: Value) -> Result<ApiKey, ApiKeyError>;

    /// Look up an API key by its hash.
    async fn get_by_hash(&self, key_hash: &str) -> Result<Option<ApiKey>, ApiKeyError>;

    /// Revoke an API key by ID.
    async fn revoke(&self, id: Uuid) -> Result<(), ApiKeyError>;

    /// List API keys for an owner.
    async fn list(&self, owner: &str) -> Result<Vec<ApiKey>, ApiKeyError>;
}

/// An API key record (never stores the raw key).
#[derive(Debug, Clone)]
pub struct ApiKey {
    pub id: Uuid,
    pub key_hash: String,
    pub owner: String,
    pub capabilities: Value,
    pub created_at: DateTime<Utc>,
    pub revoked_at: Option<DateTime<Utc>>,
}

/// Postgres-backed [`ApiKeyStore`].
#[derive(Clone)]
pub struct PostgresApiKeyStore {
    pool: PgPool,
}

impl PostgresApiKeyStore {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl ApiKeyStore for PostgresApiKeyStore {
    async fn create(&self, key_hash: &str, owner: &str, capabilities: Value) -> Result<ApiKey, ApiKeyError> {
        let row = sqlx::query!(
            r#"
            INSERT INTO api_keys (key_hash, owner, capabilities)
            VALUES ($1, $2, $3)
            RETURNING id, key_hash, owner, capabilities, created_at, revoked_at
            "#,
            key_hash,
            owner,
            capabilities
        )
        .fetch_one(&self.pool)
        .await?;

        Ok(ApiKey {
            id: row.id,
            key_hash: row.key_hash,
            owner: row.owner,
            capabilities: row.capabilities,
            created_at: row.created_at,
            revoked_at: row.revoked_at,
        })
    }

    async fn get_by_hash(&self, key_hash: &str) -> Result<Option<ApiKey>, ApiKeyError> {
        let row = sqlx::query!(
            r#"
            SELECT id, key_hash, owner, capabilities, created_at, revoked_at
            FROM api_keys
            WHERE key_hash = $1
            "#,
            key_hash
        )
        .fetch_optional(&self.pool)
        .await?;

        Ok(row.map(|row| ApiKey {
            id: row.id,
            key_hash: row.key_hash,
            owner: row.owner,
            capabilities: row.capabilities,
            created_at: row.created_at,
            revoked_at: row.revoked_at,
        }))
    }

    async fn revoke(&self, id: Uuid) -> Result<(), ApiKeyError> {
        let now = Utc::now();
        sqlx::query!(
            "UPDATE api_keys SET revoked_at = $1 WHERE id = $2",
            now,
            id
        )
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn list(&self, owner: &str) -> Result<Vec<ApiKey>, ApiKeyError> {
        let rows = sqlx::query!(
            r#"
            SELECT id, key_hash, owner, capabilities, created_at, revoked_at
            FROM api_keys
            WHERE owner = $1
            ORDER BY created_at DESC
            "#,
            owner
        )
        .fetch_all(&self.pool)
        .await?;

        Ok(rows
            .into_iter()
            .map(|row| ApiKey {
                id: row.id,
                key_hash: row.key_hash,
                owner: row.owner,
                capabilities: row.capabilities,
                created_at: row.created_at,
                revoked_at: row.revoked_at,
            })
            .collect())
    }
}