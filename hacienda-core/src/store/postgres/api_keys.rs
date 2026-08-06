//! Postgres [`ApiKeyStore`] implementation.
//!
//! Implements the real `ApiKeyStore` trait from `crate::auth` (the one `HaciendaFacade`
//! and `InMemoryApiKeyStore` already use) — this module must not define its own copy of
//! that trait or of `ApiKey`/`ApiKeyError`, or a Postgres-backed store silently stops
//! satisfying the trait bound the facade actually requires.

use crate::auth::keys::ApiKeyError;
use crate::auth::{ApiKey, ApiKeyStore, Capability};
use async_trait::async_trait;
use chrono::Utc;
use sqlx::PgPool;
use uuid::Uuid;

/// Postgres-backed [`ApiKeyStore`].
#[derive(Clone)]
pub struct PostgresApiKeyStore {
    pool: PgPool,
}

impl PostgresApiKeyStore {
    /// Create a new store from an existing pool.
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

/// Maps a `sqlx::Error` to the domain `ApiKeyError`. There is no dedicated `Database`
/// arm on `ApiKeyError` today, so this surfaces as `Verification` with the driver's
/// message — sufficient for an operator to diagnose without leaking query text to a
/// caller, since callers only ever see this via the facade's capability-checked methods.
fn map_sqlx_err(e: sqlx::Error) -> ApiKeyError {
    ApiKeyError::Verification(e.to_string())
}

#[async_trait]
impl ApiKeyStore for PostgresApiKeyStore {
    async fn create(
        &self,
        key_hash: &str,
        lookup_hash: &str,
        owner: &str,
        capabilities: Vec<Capability>,
    ) -> Result<ApiKey, ApiKeyError> {
        let capabilities_json = serde_json::to_value(&capabilities)
            .map_err(|e| ApiKeyError::Verification(e.to_string()))?;

        let row = sqlx::query!(
            r#"
            INSERT INTO api_keys (key_hash, lookup_hash, owner, capabilities)
            VALUES ($1, $2, $3, $4)
            RETURNING id, key_hash, lookup_hash, owner, capabilities, created_at, revoked_at
            "#,
            key_hash,
            lookup_hash,
            owner,
            capabilities_json
        )
        .fetch_one(&self.pool)
        .await
        .map_err(map_sqlx_err)?;

        Ok(ApiKey {
            id: row.id,
            key_hash: row.key_hash,
            lookup_hash: row.lookup_hash,
            owner: row.owner,
            capabilities: row.capabilities,
            created_at: row.created_at,
            revoked_at: row.revoked_at,
        })
    }

    async fn get_by_lookup_hash(&self, lookup_hash: &str) -> Result<Option<ApiKey>, ApiKeyError> {
        let row = sqlx::query!(
            r#"
            SELECT id, key_hash, lookup_hash, owner, capabilities, created_at, revoked_at
            FROM api_keys
            WHERE lookup_hash = $1
            "#,
            lookup_hash
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(map_sqlx_err)?;

        Ok(row.map(|row| ApiKey {
            id: row.id,
            key_hash: row.key_hash,
            lookup_hash: row.lookup_hash,
            owner: row.owner,
            capabilities: row.capabilities,
            created_at: row.created_at,
            revoked_at: row.revoked_at,
        }))
    }

    async fn revoke(&self, id: Uuid) -> Result<(), ApiKeyError> {
        let now = Utc::now();
        sqlx::query!("UPDATE api_keys SET revoked_at = $1 WHERE id = $2", now, id)
            .execute(&self.pool)
            .await
            .map_err(map_sqlx_err)?;
        Ok(())
    }

    async fn list(&self, owner: &str) -> Result<Vec<ApiKey>, ApiKeyError> {
        let rows = sqlx::query!(
            r#"
            SELECT id, key_hash, lookup_hash, owner, capabilities, created_at, revoked_at
            FROM api_keys
            WHERE owner = $1
            ORDER BY created_at DESC
            "#,
            owner
        )
        .fetch_all(&self.pool)
        .await
        .map_err(map_sqlx_err)?;

        Ok(rows
            .into_iter()
            .map(|row| ApiKey {
                id: row.id,
                key_hash: row.key_hash,
                lookup_hash: row.lookup_hash,
                owner: row.owner,
                capabilities: row.capabilities,
                created_at: row.created_at,
                revoked_at: row.revoked_at,
            })
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::postgres::test_support;

    // Ignored by default — shares one Postgres instance with the other postgres-feature
    // test modules (see `test_support::shared`), so needs `--test-threads=1`. Run with:
    //   cargo test -p hacienda-core --features postgres \
    //     --lib store::postgres::api_keys -- --ignored --test-threads=1

    async fn test_store() -> PostgresApiKeyStore {
        let pool = test_support::shared().await.pool();
        PostgresApiKeyStore::new(pool)
    }

    #[tokio::test]
    #[ignore]
    async fn should_round_trip_create_get_list_and_revoke() {
        let store = test_store().await;
        let owner = format!("owner-{}", Uuid::new_v4());
        let key_hash = format!("hash-{}", Uuid::new_v4());
        let lookup_hash = format!("lookup-{}", Uuid::new_v4());
        let capabilities = vec![Capability::DocumentsProcess, Capability::AuditRead];

        let created = store
            .create(&key_hash, &lookup_hash, &owner, capabilities.clone())
            .await
            .expect("create failed");
        assert_eq!(created.owner, owner);
        assert_eq!(created.key_hash, key_hash);
        assert_eq!(created.lookup_hash, lookup_hash);
        assert!(created.revoked_at.is_none());

        let by_lookup_hash = store
            .get_by_lookup_hash(&lookup_hash)
            .await
            .expect("get_by_lookup_hash failed")
            .expect("key must exist");
        assert_eq!(by_lookup_hash.id, created.id);

        let listed = store.list(&owner).await.expect("list failed");
        assert!(listed.iter().any(|k| k.id == created.id));

        store.revoke(created.id).await.expect("revoke failed");
        let revoked = store
            .get_by_lookup_hash(&lookup_hash)
            .await
            .expect("get_by_lookup_hash failed")
            .expect("key must still exist after revoke");
        assert!(revoked.revoked_at.is_some());
    }
}
