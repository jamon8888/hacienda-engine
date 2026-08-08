//! Postgres [`ApiKeyStore`] implementation.
//!
//! Implements the real `ApiKeyStore` trait from `crate::auth` (the one `HaciendaFacade`
//! and `InMemoryApiKeyStore` already use) — this module must not define its own copy of
//! that trait or of `ApiKey`/`ApiKeyError`, or a Postgres-backed store silently stops
//! satisfying the trait bound the facade actually requires.

use crate::auth::keys::ApiKeyError;
use crate::auth::{ApiKey, ApiKeyStore, Capability};
use crate::tenancy::TenantId;
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
        tenant: &TenantId,
        capabilities: Vec<Capability>,
    ) -> Result<ApiKey, ApiKeyError> {
        let capabilities_json =
            serde_json::to_value(&capabilities).map_err(ApiKeyError::CapabilitySerialization)?;
        let tenant_str = tenant.as_str();

        let row = sqlx::query!(
            r#"
            INSERT INTO api_keys (key_hash, lookup_hash, owner, tenant_id, capabilities)
            VALUES ($1, $2, $3, $4, $5)
            RETURNING id, key_hash, lookup_hash, owner, tenant_id, capabilities, created_at, revoked_at
            "#,
            key_hash,
            lookup_hash,
            owner,
            tenant_str,
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
            tenant: TenantId::new(row.tenant_id),
            capabilities: row.capabilities,
            created_at: row.created_at,
            revoked_at: row.revoked_at,
        })
    }

    async fn get_by_lookup_hash(&self, lookup_hash: &str) -> Result<Option<ApiKey>, ApiKeyError> {
        let row = sqlx::query!(
            r#"
            SELECT id, key_hash, lookup_hash, owner, tenant_id, capabilities, created_at, revoked_at
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
            tenant: TenantId::new(row.tenant_id),
            capabilities: row.capabilities,
            created_at: row.created_at,
            revoked_at: row.revoked_at,
        }))
    }

    async fn revoke(&self, id: Uuid, tenant: &TenantId) -> Result<(), ApiKeyError> {
        let now = Utc::now();
        let tenant_str = tenant.as_str();
        sqlx::query!(
            "UPDATE api_keys SET revoked_at = $1 WHERE id = $2 AND tenant_id = $3",
            now,
            id,
            tenant_str
        )
        .execute(&self.pool)
        .await
        .map_err(map_sqlx_err)?;
        Ok(())
    }

    async fn list(&self, tenant: &TenantId, owner: &str) -> Result<Vec<ApiKey>, ApiKeyError> {
        let tenant_str = tenant.as_str();
        let rows = sqlx::query!(
            r#"
            SELECT id, key_hash, lookup_hash, owner, tenant_id, capabilities, created_at, revoked_at
            FROM api_keys
            WHERE tenant_id = $1 AND owner = $2
            ORDER BY created_at DESC
            "#,
            tenant_str,
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
                tenant: TenantId::new(row.tenant_id),
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

    #[test]
    #[ignore]
    fn should_round_trip_create_get_list_and_revoke() {
        test_support::block_on_shared(async {
            let store = test_store().await;
            let owner = format!("owner-{}", Uuid::new_v4());
            let key_hash = format!("hash-{}", Uuid::new_v4());
            let lookup_hash = format!("lookup-{}", Uuid::new_v4());
            let capabilities = vec![Capability::DocumentsProcess, Capability::AuditRead];

            let tenant = TenantId::new(format!("tenant-{}", Uuid::new_v4()));

            let created = store
                .create(
                    &key_hash,
                    &lookup_hash,
                    &owner,
                    &tenant,
                    capabilities.clone(),
                )
                .await
                .expect("create failed");
            assert_eq!(created.owner, owner);
            assert_eq!(created.key_hash, key_hash);
            assert_eq!(created.lookup_hash, lookup_hash);
            assert_eq!(created.tenant, tenant);
            assert!(created.revoked_at.is_none());

            let by_lookup_hash = store
                .get_by_lookup_hash(&lookup_hash)
                .await
                .expect("get_by_lookup_hash failed")
                .expect("key must exist");
            assert_eq!(by_lookup_hash.id, created.id);
            assert_eq!(by_lookup_hash.tenant, tenant);

            let listed = store.list(&tenant, &owner).await.expect("list failed");
            assert!(listed.iter().any(|k| k.id == created.id));

            let other_tenant = TenantId::new(format!("tenant-{}", Uuid::new_v4()));
            let listed_other_tenant = store
                .list(&other_tenant, &owner)
                .await
                .expect("list failed");
            assert!(
                !listed_other_tenant.iter().any(|k| k.id == created.id),
                "a key must never be visible from a different tenant, even with the same owner"
            );

            store
                .revoke(created.id, &other_tenant)
                .await
                .expect("revoke must not error even for another tenant's key id");
            let not_revoked = store
                .get_by_lookup_hash(&lookup_hash)
                .await
                .expect("get_by_lookup_hash failed")
                .expect("key must still exist after a cross-tenant revoke attempt");
            assert!(
                not_revoked.revoked_at.is_none(),
                "a cross-tenant revoke attempt must not revoke the key"
            );

            store
                .revoke(created.id, &tenant)
                .await
                .expect("revoke failed");
            let revoked = store
                .get_by_lookup_hash(&lookup_hash)
                .await
                .expect("get_by_lookup_hash failed")
                .expect("key must still exist after revoke");
            assert!(revoked.revoked_at.is_some());
        });
    }
}
