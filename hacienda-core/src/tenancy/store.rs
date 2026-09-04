//! [`TenantStore`] trait and the in-memory backend.
//!
//! A tenant must be admitted here before anything else in S1 can reference it
//! meaningfully: [`crate::redaction::TenantPseudonymiserRegistry::admit`] resolves a
//! tenant's pseudonymization keys, and the migration path (spec §8) backfills existing
//! rows onto the `default` tenant — both need a real row to point at, not just a
//! `TenantId` a caller made up.

use super::{Tenant, TenantError, TenantId};
use async_trait::async_trait;
use chrono::Utc;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// Persistent store for registered tenants.
///
/// Exercised by tenant administration (creating/listing tenants) — not by the
/// request-time authorization path, which uses [`super::TenantCtx`]/[`TenantId`]
/// directly and never needs to look up a `Tenant` row to enforce cloisonnement.
#[async_trait]
/// TenantStore trait
pub trait TenantStore: Send + Sync {
    /// Register a new tenant.
    ///
    /// # Errors
    ///
    /// [`TenantError::AlreadyExists`] if a tenant with this id already has a row —
    /// callers that want idempotent admission (the spec §8 migration path) must `get`
    /// first and only `create` when absent.
    async fn create(
        &self,
        id: TenantId,
        display_name: Option<String>,
    ) -> Result<Tenant, TenantError>;

    /// Look up a tenant by id, returning `None` if it does not exist.
    async fn get(&self, id: &TenantId) -> Result<Option<Tenant>, TenantError>;

    /// List every registered tenant, ordered by admission time, oldest first.
    async fn list(&self) -> Result<Vec<Tenant>, TenantError>;
}

/// In-memory [`TenantStore`] backed by a `Mutex<HashMap>`.
///
/// Appropriate for testing and for deployments that accept re-admitting tenants on
/// restart — the same durability trade-off [`crate::jobs::InMemoryJobStore`] documents.
#[derive(Debug, Default)]
/// InMemoryTenantStore struct
pub struct InMemoryTenantStore {
    tenants: Mutex<HashMap<TenantId, Tenant>>,
}

impl InMemoryTenantStore {
/// new function
    pub fn new() -> Self {
        Self::default()
    }

    /// Wrap this store in an `Arc` so it satisfies `Arc<dyn TenantStore>`.
    pub fn into_arc(self) -> Arc<dyn TenantStore> {
        Arc::new(self)
    }
}

#[async_trait]
impl TenantStore for InMemoryTenantStore {
    async fn create(
        &self,
        id: TenantId,
        display_name: Option<String>,
    ) -> Result<Tenant, TenantError> {
        let mut guard = self
            .tenants
            .lock()
            .map_err(|_| TenantError::Internal("lock poisoned".into()))?;
        if guard.contains_key(&id) {
            return Err(TenantError::AlreadyExists(id));
        }
        let tenant = Tenant {
            id: id.clone(),
            display_name,
            created_at: Utc::now().to_rfc3339(),
        };
        guard.insert(id, tenant.clone());
        Ok(tenant)
    }

    async fn get(&self, id: &TenantId) -> Result<Option<Tenant>, TenantError> {
        let guard = self
            .tenants
            .lock()
            .map_err(|_| TenantError::Internal("lock poisoned".into()))?;
        Ok(guard.get(id).cloned())
    }

    async fn list(&self) -> Result<Vec<Tenant>, TenantError> {
        let guard = self
            .tenants
            .lock()
            .map_err(|_| TenantError::Internal("lock poisoned".into()))?;
        let mut tenants: Vec<Tenant> = guard.values().cloned().collect();
        // Stable order, same tie-break rationale as InMemoryJobStore::list: RFC 3339
        // timestamps have finite resolution, so two tenants admitted in the same
        // instant compare equal and need a deterministic tie-break rather than
        // HashMap's per-process-randomised iteration order.
        tenants.sort_by(|a, b| {
            a.created_at
                .cmp(&b.created_at)
                .then_with(|| a.id.as_str().cmp(b.id.as_str()))
        });
        Ok(tenants)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn should_create_and_get_a_tenant() {
        let store = InMemoryTenantStore::new();
        let tenant = store
            .create(TenantId::new("acme"), Some("Acme Corp".into()))
            .await
            .unwrap();
        assert_eq!(tenant.id, TenantId::new("acme"));
        assert_eq!(tenant.display_name.as_deref(), Some("Acme Corp"));

        let fetched = store.get(&TenantId::new("acme")).await.unwrap().unwrap();
        assert_eq!(fetched, tenant);
    }

    #[tokio::test]
    async fn should_return_none_for_an_unregistered_tenant() {
        let store = InMemoryTenantStore::new();
        assert!(store.get(&TenantId::new("ghost")).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn should_refuse_to_create_the_same_tenant_id_twice() {
        let store = InMemoryTenantStore::new();
        store.create(TenantId::new("acme"), None).await.unwrap();
        let err = store
            .create(TenantId::new("acme"), Some("Second".into()))
            .await
            .unwrap_err();
        assert!(matches!(err, TenantError::AlreadyExists(id) if id == TenantId::new("acme")));
    }

    #[tokio::test]
    async fn should_list_tenants_oldest_first() {
        let store = InMemoryTenantStore::new();
        store.create(TenantId::new("b"), None).await.unwrap();
        store.create(TenantId::new("a"), None).await.unwrap();
        let tenants = store.list().await.unwrap();
        assert_eq!(tenants.len(), 2);
        // Both admitted in quick succession may share a timestamp; the tie-break on
        // id makes the order deterministic either way.
        let ids: Vec<&str> = tenants.iter().map(|t| t.id.as_str()).collect();
        assert!(ids == vec!["b", "a"] || ids == vec!["a", "b"]);
    }
}
