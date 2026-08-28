//! Authentication and authorization for hacienda.
//!
//! Capability model per §7 of the CLI/API integration design:
//! - `documents:process` — normal extraction + redaction
//! - `pii:reveal` — `include_text=true` on scan; raw span access
//! - `audit:read` / `audit:export` — audit trail access
//! - `review:decide` — approving or rejecting detections
//! - `auth:manage` — API key issuance and revocation
//! - `raw:extract` — `/xberg/v1/*` when passthrough is compiled in

pub mod authn;
pub mod authz;
pub mod keys;

pub use crate::auth::keys::ApiKey;
use crate::tenancy::{ActorId, TenantCtx, TenantId};
use async_trait::async_trait;

use serde::{Deserialize, Serialize};
use std::collections::HashSet;

/// A capability that guards access to a hacienda operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
/// Capability enum
pub enum Capability {
    /// Normal extraction with redaction. The default for content-bearing endpoints.
    DocumentsProcess,
    /// Reveal span text on scan; access raw PII values.
    PiiReveal,
    /// Read audit entries and verify the chain.
    AuditRead,
    /// Export audit chain (JSON, CSV, JSONL).
    AuditExport,
    /// Decide review items (approve/reject/modify).
    ReviewDecide,
    /// Manage API keys (issue, revoke, list).
    AuthManage,
    /// Raw xberg passthrough (unredacted). Only available with `xberg-passthrough` feature.
    RawExtract,
}

impl Capability {
    /// All capabilities.
    pub fn all() -> Vec<Self> {
        vec![
            Self::DocumentsProcess,
            Self::PiiReveal,
            Self::AuditRead,
            Self::AuditExport,
            Self::ReviewDecide,
            Self::AuthManage,
            Self::RawExtract,
        ]
    }
}

impl std::fmt::Display for Capability {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::DocumentsProcess => write!(f, "documents:process"),
            Self::PiiReveal => write!(f, "pii:reveal"),
            Self::AuditRead => write!(f, "audit:read"),
            Self::AuditExport => write!(f, "audit:export"),
            Self::ReviewDecide => write!(f, "review:decide"),
            Self::AuthManage => write!(f, "auth:manage"),
            Self::RawExtract => write!(f, "raw:extract"),
        }
    }
}

impl std::str::FromStr for Capability {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "documents:process" => Ok(Self::DocumentsProcess),
            "pii:reveal" => Ok(Self::PiiReveal),
            "audit:read" => Ok(Self::AuditRead),
            "audit:export" => Ok(Self::AuditExport),
            "review:decide" => Ok(Self::ReviewDecide),
            "auth:manage" => Ok(Self::AuthManage),
            "raw:extract" => Ok(Self::RawExtract),
            _ => Err(format!("unknown capability: {s}")),
        }
    }
}

/// A set of capabilities granted to a principal.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
/// CapabilitySet struct
pub struct CapabilitySet(HashSet<Capability>);

impl CapabilitySet {
/// new function
    pub fn new(capabilities: impl IntoIterator<Item = Capability>) -> Self {
        Self(capabilities.into_iter().collect())
    }

/// empty function
    pub fn empty() -> Self {
        Self::default()
    }

/// all function
    pub fn all() -> Self {
        Self(Capability::all().into_iter().collect())
    }

/// has function
    pub fn has(&self, cap: Capability) -> bool {
        self.0.contains(&cap)
    }

/// insert function
    pub fn insert(&mut self, cap: Capability) {
        self.0.insert(cap);
    }

/// remove function
    pub fn remove(&mut self, cap: Capability) {
        self.0.remove(&cap);
    }

/// iter function
    pub fn iter(&self) -> impl Iterator<Item = Capability> + '_ {
        self.0.iter().copied()
    }

/// is_empty function
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

/// len function
    pub fn len(&self) -> usize {
        self.0.len()
    }
}

impl std::ops::Deref for CapabilitySet {
    type Target = HashSet<Capability>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl FromIterator<Capability> for CapabilitySet {
    fn from_iter<T: IntoIterator<Item = Capability>>(iter: T) -> Self {
        Self(iter.into_iter().collect())
    }
}

impl IntoIterator for CapabilitySet {
    type Item = Capability;
    type IntoIter = std::collections::hash_set::IntoIter<Capability>;

    fn into_iter(self) -> Self::IntoIter {
        self.0.into_iter()
    }
}

/// Authentication context for a request.
#[derive(Debug, Clone, Serialize, Deserialize)]
/// AuthContext struct
pub struct AuthContext {
    /// Unique identifier for the principal (user, service, etc.).
    pub principal_id: String,
    /// Human-readable name or description.
    pub principal_name: Option<String>,
    /// Capabilities granted to this principal.
    pub capabilities: CapabilitySet,
    /// The tenant this principal belongs to (S1). Every capability this context grants
    /// is scoped to this tenant — a principal does not carry capabilities across tenants.
    pub tenant: TenantId,
    /// Optional metadata (roles, etc.).
    pub metadata: serde_json::Value,
}

impl AuthContext {
    /// Create a new auth context with the given principal ID and capabilities, scoped to
    /// the default tenant.
    ///
    /// Most existing call sites (single-tenant deployments, tests) want this. A principal
    /// resolved from a real, multi-tenant-aware store should use
    /// [`Self::with_tenant`] instead so its capabilities are scoped to its actual tenant.
    pub fn new(principal_id: impl Into<String>, capabilities: CapabilitySet) -> Self {
        Self::with_tenant(principal_id, TenantId::default_tenant(), capabilities)
    }

    /// Create a new auth context scoped to an explicit tenant.
    pub fn with_tenant(
        principal_id: impl Into<String>,
        tenant: TenantId,
        capabilities: CapabilitySet,
    ) -> Self {
        Self {
            principal_id: principal_id.into(),
            principal_name: None,
            capabilities,
            tenant,
            metadata: serde_json::Value::Null,
        }
    }

    /// Check if this context has the given capability.
    pub fn has_capability(&self, cap: Capability) -> bool {
        self.capabilities.has(cap)
    }

    /// Require the given capability, returning an error if not present.
    pub fn require_capability(&self, cap: Capability) -> Result<(), AuthzError> {
        if self.has_capability(cap) {
            Ok(())
        } else {
            Err(AuthzError::MissingCapability {
                required: cap,
                principal: self.principal_id.clone(),
            })
        }
    }
}

/// Who is asking, for capability checks on the core API.
///
/// This exists instead of `Option<&AuthContext>` so that skipping the check is a named
/// decision rather than an omitted argument. `None` is invisible at a call site and
/// indistinguishable from "I forgot to thread the context through"; `Caller::Trusted`
/// states the intent, and `git grep Caller::Trusted` enumerates every bypass in the
/// codebase — which is the review question §7 actually cares about.
#[derive(Debug, Clone, Copy)]
/// Caller enum
pub enum Caller<'a> {
    /// An in-process caller: the CLI, the desktop app, a test. The process boundary is
    /// the trust boundary, so no capability is enforced.
    Trusted,
    /// An authenticated principal. Every capability the operation names is enforced.
    Principal(&'a AuthContext),
}

impl<'a> Caller<'a> {
    /// Enforce `cap`, unless this caller is in-process.
    pub fn require(&self, cap: Capability) -> Result<(), AuthzError> {
        match self {
            Self::Trusted => Ok(()),
            Self::Principal(ctx) => ctx.require_capability(cap),
        }
    }

    /// The principal's id, for audit records. `None` for in-process callers.
    pub fn principal_id(&self) -> Option<&str> {
        match self {
            Self::Trusted => None,
            Self::Principal(ctx) => Some(&ctx.principal_id),
        }
    }

    /// The tenant-scoping context (S1) this caller acts under.
    ///
    /// `Principal` callers carry their real, resolved tenant. `Trusted` (in-process)
    /// callers have no tenant of their own to assert, so this resolves to the default
    /// tenant — the same one every pre-S1 row is migrated onto (spec §8). A deployment
    /// that wants an in-process caller scoped to a *specific* tenant (e.g. a CLI operator
    /// managing one tenant among several) does not yet have a way to express that; no
    /// caller in this codebase needs it today, so it is left as a known gap rather than
    /// guessed at.
    pub fn tenant_ctx(&self) -> TenantCtx {
        match self {
            Self::Trusted => TenantCtx::default_tenant(ActorId::new("trusted")),
            Self::Principal(ctx) => {
                TenantCtx::new(ctx.tenant.clone(), ActorId::new(ctx.principal_id.clone()))
            }
        }
    }
}

impl<'a> From<&'a AuthContext> for Caller<'a> {
    fn from(ctx: &'a AuthContext) -> Self {
        Self::Principal(ctx)
    }
}

/// Authentication/authorization errors.
#[derive(Debug, thiserror::Error)]
/// AuthzError enum
pub enum AuthzError {
    #[error("authentication required")]
    Unauthenticated,
    #[error("capability {required} required for principal {principal}")]
    MissingCapability {
        required: Capability,
        principal: String,
    },
    #[error("invalid token: {0}")]
    InvalidToken(String),
    #[error("token expired")]
    TokenExpired,
    #[error("issuer not trusted: {0}")]
    UntrustedIssuer(String),
}

/// Trait for resolving a bearer token to a capability set.
///
/// Used by the auth middleware to authenticate requests. Implementations
/// must hash the incoming token and look up the stored hash — never store
/// or compare raw keys.
#[async_trait]
pub trait TokenResolver: Send + Sync {
    /// Resolve a bearer token to its capability set.
    ///
    /// Returns `None` if the token is not found, revoked, or malformed.
    async fn resolve(&self, bearer_token: &str) -> Option<CapabilitySet>;
}

/// Trait for API key storage.
///
/// All methods are async so that database backends can perform I/O without
/// blocking the async runtime. The in-memory backend does no I/O but uses
/// the same signatures, so callers never need to know which backend is present.
///
/// Implementations must be `Send + Sync` because the store lives behind an `Arc`
/// shared across tasks.
#[async_trait]
pub trait ApiKeyStore: Send + Sync {
    /// Create a new API key record.
    ///
    /// `key_hash` is the Argon2id hash (verification only — see
    /// `crate::auth::keys` module docs for why it cannot double as a lookup key).
    /// `lookup_hash` is the deterministic BLAKE3 digest used by
    /// [`Self::get_by_lookup_hash`] to find this record from a presented key.
    async fn create(
        &self,
        ctx: &crate::tenancy::TenantCtx,
        key_hash: &str,
        lookup_hash: &str,
        owner: &str,
        capabilities: Vec<Capability>,
    ) -> Result<ApiKey, crate::auth::keys::ApiKeyError>;

    /// Look up an API key by its deterministic lookup digest (see
    /// [`crate::auth::keys::lookup_key`]).
    ///
    /// This is *not* a lookup by `key_hash` — Argon2id salts every hash it produces, so a
    /// freshly computed `key_hash` for a presented key never matches the stored one.
    /// Callers must still call `crate::auth::keys::verify_key` against the returned
    /// record's `key_hash` before trusting the match.
    ///
    /// Deliberately **not** scoped by tenant (S1): a presented raw key is how a caller's
    /// tenant is discovered in the first place, so this must search across every tenant.
    async fn get_by_lookup_hash(
        &self,
        lookup_hash: &str,
    ) -> Result<Option<ApiKey>, crate::auth::keys::ApiKeyError>;

    /// Revoke an API key by ID, scoped to `ctx`'s tenant (S1).
    ///
    /// Revoking an id that does not exist, was already revoked, or belongs to a
    /// different tenant are all indistinguishable no-ops — same idempotency contract as
    /// before S1, now also closing a cross-tenant gap: without the tenant filter, any
    /// caller holding `auth:manage` could revoke another tenant's key by guessing its
    /// id, since ids are never exposed to non-owners. See `crate::tenancy` module docs,
    /// D-S1-6 (cross-tenant absence looks identical to non-existence).
    async fn revoke(
        &self,
        ctx: &crate::tenancy::TenantCtx,
        id: uuid::Uuid,
    ) -> Result<(), crate::auth::keys::ApiKeyError>;

    /// List API keys for an owner within `ctx`'s tenant (S1).
    async fn list(
        &self,
        ctx: &crate::tenancy::TenantCtx,
        owner: &str,
    ) -> Result<Vec<ApiKey>, crate::auth::keys::ApiKeyError>;
}

/// In-memory implementation of [`ApiKeyStore`] for testing.
#[derive(Debug, Default)]
/// InMemoryApiKeyStore struct
pub struct InMemoryApiKeyStore {
    keys: std::sync::Mutex<std::collections::HashMap<String, ApiKey>>,
}

impl InMemoryApiKeyStore {
/// new function
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait]
impl ApiKeyStore for InMemoryApiKeyStore {
    async fn create(
        &self,
        ctx: &crate::tenancy::TenantCtx,
        key_hash: &str,
        lookup_hash: &str,
        owner: &str,
        capabilities: Vec<Capability>,
    ) -> Result<ApiKey, crate::auth::keys::ApiKeyError> {
        let id = uuid::Uuid::new_v4();
        let now = chrono::Utc::now();
        let key = ApiKey {
            id,
            key_hash: key_hash.to_string(),
            lookup_hash: lookup_hash.to_string(),
            owner: owner.to_string(),
            capabilities: serde_json::to_value(capabilities).unwrap(),
            created_at: now,
            revoked_at: None,
            tenant_id: ctx.tenant.to_string(),
        };
        // Keyed by `lookup_hash`, the deterministic digest — `key_hash` is salted and
        // would never match a freshly hashed presented key. See the `keys` module docs.
        self.keys
            .lock()
            .unwrap()
            .insert(lookup_hash.to_string(), key.clone());
        Ok(key)
    }

    async fn get_by_lookup_hash(
        &self,
        lookup_hash: &str,
    ) -> Result<Option<ApiKey>, crate::auth::keys::ApiKeyError> {
        Ok(self.keys.lock().unwrap().get(lookup_hash).cloned())
    }

    async fn revoke(
        &self,
        ctx: &crate::tenancy::TenantCtx,
        id: uuid::Uuid,
    ) -> Result<(), crate::auth::keys::ApiKeyError> {
        let mut keys = self.keys.lock().unwrap();
        if let Some(key) = keys
            .values_mut()
            .find(|k| k.id == id && k.tenant_id == ctx.tenant.as_str())
        {
            key.revoked_at = Some(chrono::Utc::now());
        }
        Ok(())
    }

    async fn list(
        &self,
        ctx: &crate::tenancy::TenantCtx,
        owner: &str,
    ) -> Result<Vec<ApiKey>, crate::auth::keys::ApiKeyError> {
        Ok(self
            .keys
            .lock()
            .unwrap()
            .values()
            .filter(|k| k.owner == owner && k.tenant_id == ctx.tenant.as_str())
            .cloned()
            .collect())
    }
}

// Re-export authz types
pub use authz::{AuthExtension, AuthRouterBuilder, AuthState};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capability_display_and_parse_roundtrip() {
        for cap in Capability::all() {
            let s = cap.to_string();
            let parsed = s.parse::<Capability>().unwrap();
            assert_eq!(cap, parsed);
        }
    }

    #[test]
    fn capability_set_operations() {
        let mut set = CapabilitySet::empty();
        assert!(set.is_empty());

        set.insert(Capability::DocumentsProcess);
        assert!(set.has(Capability::DocumentsProcess));
        assert!(!set.has(Capability::PiiReveal));

        set.insert(Capability::PiiReveal);
        assert_eq!(set.len(), 2);

        set.remove(Capability::DocumentsProcess);
        assert!(!set.has(Capability::DocumentsProcess));
        assert!(set.has(Capability::PiiReveal));
    }

    #[test]
    fn auth_context_capability_check() {
        let ctx = AuthContext::new(
            "user-123",
            CapabilitySet::new([Capability::DocumentsProcess, Capability::AuditRead]),
        );

        assert!(ctx.has_capability(Capability::DocumentsProcess));
        assert!(ctx.has_capability(Capability::AuditRead));
        assert!(!ctx.has_capability(Capability::PiiReveal));

        assert!(ctx.require_capability(Capability::DocumentsProcess).is_ok());
        assert!(ctx.require_capability(Capability::PiiReveal).is_err());
    }

    #[test]
    fn trusted_caller_bypasses_and_principal_caller_enforces() {
        assert!(Caller::Trusted.require(Capability::RawExtract).is_ok());
        assert_eq!(Caller::Trusted.principal_id(), None);

        let ctx = AuthContext::new("user-123", CapabilitySet::new([Capability::AuditRead]));
        let caller = Caller::from(&ctx);
        assert!(caller.require(Capability::AuditRead).is_ok());
        assert!(matches!(
            caller.require(Capability::RawExtract),
            Err(AuthzError::MissingCapability { .. })
        ));
        assert_eq!(caller.principal_id(), Some("user-123"));
    }

    #[test]
    fn auth_context_new_defaults_to_the_default_tenant() {
        let ctx = AuthContext::new("user-123", CapabilitySet::empty());
        assert_eq!(ctx.tenant, crate::tenancy::TenantId::default_tenant());
    }

    #[test]
    fn auth_context_with_tenant_carries_the_given_tenant() {
        let ctx = AuthContext::with_tenant(
            "user-123",
            crate::tenancy::TenantId::new("acme"),
            CapabilitySet::empty(),
        );
        assert_eq!(ctx.tenant, crate::tenancy::TenantId::new("acme"));
    }

    #[test]
    fn trusted_caller_tenant_ctx_is_the_default_tenant() {
        let ctx = Caller::Trusted.tenant_ctx();
        assert_eq!(ctx.tenant, crate::tenancy::TenantId::default_tenant());
    }

    #[test]
    fn principal_caller_tenant_ctx_carries_the_principals_tenant_and_id() {
        let auth_ctx = AuthContext::with_tenant(
            "user-123",
            crate::tenancy::TenantId::new("acme"),
            CapabilitySet::empty(),
        );
        let caller = Caller::from(&auth_ctx);
        let tenant_ctx = caller.tenant_ctx();
        assert_eq!(tenant_ctx.tenant, crate::tenancy::TenantId::new("acme"));
        assert_eq!(tenant_ctx.actor, crate::tenancy::ActorId::new("user-123"));
    }

    #[test]
    fn two_principals_in_different_tenants_get_different_tenant_ctx() {
        let acme = AuthContext::with_tenant(
            "user-1",
            crate::tenancy::TenantId::new("acme"),
            CapabilitySet::empty(),
        );
        let globex = AuthContext::with_tenant(
            "user-2",
            crate::tenancy::TenantId::new("globex"),
            CapabilitySet::empty(),
        );
        assert_ne!(
            Caller::from(&acme).tenant_ctx().tenant,
            Caller::from(&globex).tenant_ctx().tenant
        );
    }
}
