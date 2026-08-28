//! Tenant scoping — the cloisonnement boundary defined by S1
//! (`superpowers/specs/2026-08-01-S1-tenancy-and-projects.md`).
//!
//! [`TenantCtx`] is threaded as an **explicit parameter** into every store method that
//! reads or writes tenant-scoped data (decision D-S1-1). A store built *for* one tenant
//! invites one store instance per tenant, which multiplies connection pools and lets
//! scoping be forgotten at the factory — where it is invisible. A parameter makes the
//! omission fail to compile once a call site is migrated to a tenant-scoped signature.
//!
//! Only [`TenantId`] is a security boundary. [`ProjectId`] organizes; it does not
//! cloister (decision D-S1-2) — treating it as a boundary would create a second,
//! weaker, invisible authorization layer.
//!
//! The types on this page ([`TenantId`], [`ActorId`], [`ProjectId`], [`TenantCtx`]) are
//! plain `String` newtypes with no native-only dependencies, so they compile for
//! `wasm32` — `redaction::pseudonym`, which is explicitly wasm-safe, depends on
//! [`TenantCtx`]. [`store`] (the [`TenantStore`](store::TenantStore) trait and its
//! backends) is server-side only and gated accordingly.

use serde::{Deserialize, Serialize};

#[cfg(not(target_arch = "wasm32"))]
/// Module error
pub mod error;
#[cfg(not(target_arch = "wasm32"))]
/// Module store
pub mod store;
#[cfg(not(target_arch = "wasm32"))]
/// Module types
pub mod types;

#[cfg(not(target_arch = "wasm32"))]
pub use error::TenantError;
#[cfg(not(target_arch = "wasm32"))]
pub use store::{InMemoryTenantStore, TenantStore};
#[cfg(not(target_arch = "wasm32"))]
pub use types::Tenant;

/// The identifier every store-level cloisonnement check keys off.
///
/// Two tenants are never comparable for access purposes: a resource that does not belong
/// to the calling tenant is reported as absent (404), never forbidden (403) — see
/// decision D-S1-6. `TenantId` is intentionally opaque (no ordering, no arithmetic) so
/// nothing can be tempted to derive one tenant's identifier from another's.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TenantId(String);

impl TenantId {
    /// Wrap an existing identifier (e.g. one read back from a store row).
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    /// The identifier for the single tenant that owns every row created before S1 shipped
    /// (spec §8's migration path). There is, by construction, only ever one deployment
    /// migrating into this identifier — an existing single-tenant install.
    pub fn default_tenant() -> Self {
        Self::new(DEFAULT_TENANT)
    }

    /// The underlying identifier string.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for TenantId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// The tenant identifier the pre-S1, single-tenant deployment is migrated onto. Rows
/// with no recorded tenant are backfilled to this value exactly once (spec §8) — the
/// migration is idempotent and only ever meaningful for a single pre-existing tenant.
pub const DEFAULT_TENANT: &str = "default";

/// The principal acting within a tenant — distinct from [`crate::auth::AuthContext`]'s
/// `principal_id` only in name, kept as its own type so a future actor representation
/// (service accounts acting on behalf of a user, for instance) does not require touching
/// every [`TenantCtx`] call site.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ActorId(String);

impl ActorId {
    /// Wrap an existing identifier (e.g. one read back from a store row).
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    /// The underlying identifier string.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for ActorId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// An optional sub-division within a tenant, for parity with Xberg Enterprise's
/// `project`. **Not a security boundary** — see the module doc and decision D-S1-2.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ProjectId(String);

impl ProjectId {
    /// Wrap an existing identifier (e.g. one read back from a store row).
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    /// The underlying identifier string.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for ProjectId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Cloisonnement identity for the duration of one request. Immutable once built.
///
/// Every tenant-scoped store method takes `&TenantCtx` as a parameter (D-S1-1) rather
/// than having it captured at construction time.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TenantCtx {
    /// The tenant this context is scoped to — the cloisonnement boundary.
    pub tenant: TenantId,
    /// The principal acting within `tenant`.
    pub actor: ActorId,
    /// See the module doc: organizes, does not cloister.
    pub project: Option<ProjectId>,
}

impl TenantCtx {
    /// Build a context scoped to `tenant`, acting as `actor`, with no project.
    pub fn new(tenant: TenantId, actor: ActorId) -> Self {
        Self {
            tenant,
            actor,
            project: None,
        }
    }

    /// Attach a project. Does not change what this context is authorized to reach —
    /// only `tenant` does that (D-S1-2).
    pub fn with_project(mut self, project: ProjectId) -> Self {
        self.project = Some(project);
        self
    }

    /// The context every pre-S1 row is migrated onto (spec §8). Also the context an
    /// in-process [`crate::auth::Caller::Trusted`] caller acts under today, since a
    /// trusted caller has no tenant of its own to assert — see
    /// `Caller::tenant_ctx`'s doc comment for the open question this leaves for
    /// deployments that want a trusted caller scoped to a specific tenant.
    pub fn default_tenant(actor: ActorId) -> Self {
        Self::new(TenantId::default_tenant(), actor)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tenant_ctx_default_tenant_has_no_project() {
        let ctx = TenantCtx::default_tenant(ActorId::new("trusted"));
        assert_eq!(ctx.tenant, TenantId::default_tenant());
        assert_eq!(ctx.actor, ActorId::new("trusted"));
        assert_eq!(ctx.project, None);
    }

    #[test]
    fn tenant_ctx_with_project_round_trips() {
        let ctx = TenantCtx::new(TenantId::new("acme"), ActorId::new("user-1"))
            .with_project(ProjectId::new("proj-1"));
        assert_eq!(ctx.project, Some(ProjectId::new("proj-1")));
    }

    #[test]
    fn two_tenant_ids_from_the_same_string_are_equal() {
        assert_eq!(TenantId::new("acme"), TenantId::new("acme"));
        assert_ne!(TenantId::new("acme"), TenantId::new("other"));
    }
}
