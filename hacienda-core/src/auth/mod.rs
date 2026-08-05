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

use async_trait::async_trait;
use crate::auth::keys::{ApiKey, ApiKeyError};

use serde::{Deserialize, Serialize};
use std::collections::HashSet;

/// A capability that guards access to a hacienda operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
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
pub struct CapabilitySet(HashSet<Capability>);

impl CapabilitySet {
    pub fn new(capabilities: impl IntoIterator<Item = Capability>) -> Self {
        Self(capabilities.into_iter().collect())
    }

    pub fn empty() -> Self {
        Self::default()
    }

    pub fn all() -> Self {
        Self(Capability::all().into_iter().collect())
    }

    pub fn has(&self, cap: Capability) -> bool {
        self.0.contains(&cap)
    }

    pub fn insert(&mut self, cap: Capability) {
        self.0.insert(cap);
    }

    pub fn remove(&mut self, cap: Capability) {
        self.0.remove(&cap);
    }

    pub fn iter(&self) -> impl Iterator<Item = Capability> + '_ {
        self.0.iter().copied()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

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
pub struct AuthContext {
    /// Unique identifier for the principal (user, service, etc.).
    pub principal_id: String,
    /// Human-readable name or description.
    pub principal_name: Option<String>,
    /// Capabilities granted to this principal.
    pub capabilities: CapabilitySet,
    /// Optional metadata (tenant, roles, etc.).
    pub metadata: serde_json::Value,
}

impl AuthContext {
    /// Create a new auth context with the given principal ID and capabilities.
    pub fn new(principal_id: impl Into<String>, capabilities: CapabilitySet) -> Self {
        Self {
            principal_id: principal_id.into(),
            principal_name: None,
            capabilities,
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
}

impl<'a> From<&'a AuthContext> for Caller<'a> {
    fn from(ctx: &'a AuthContext) -> Self {
        Self::Principal(ctx)
    }
}

/// Authentication/authorization errors.
#[derive(Debug, thiserror::Error)]
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
    /// Create a new API key record (stores hash only).
    async fn create(&self, key_hash: &str, owner: &str, capabilities: Vec<Capability>)
        -> Result<ApiKey, crate::auth::keys::ApiKeyError>;

    /// Look up an API key by its hash.
    async fn get_by_hash(&self, key_hash: &str) -> Result<Option<ApiKey>, crate::auth::keys::ApiKeyError>;

    /// Revoke an API key by ID.
    async fn revoke(&self, id: uuid::Uuid) -> Result<(), crate::auth::keys::ApiKeyError>;

    /// List API keys for an owner.
    async fn list(&self, owner: &str) -> Result<Vec<ApiKey>, crate::auth::keys::ApiKeyError>;
}

/// In-memory implementation of [`ApiKeyStore`] for testing.
#[derive(Debug, Default)]
pub struct InMemoryApiKeyStore {
    keys: std::sync::Mutex<std::collections::HashMap<String, ApiKey>>,
}

impl InMemoryApiKeyStore {
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait]
impl ApiKeyStore for InMemoryApiKeyStore {
    async fn create(
        &self,
        key_hash: &str,
        owner: &str,
        capabilities: Vec<Capability>,
    ) -> Result<ApiKey, crate::auth::keys::ApiKeyError> {
        let id = uuid::Uuid::new_v4();
        let now = chrono::Utc::now();
        let key = ApiKey {
            id,
            key_hash: key_hash.to_string(),
            owner: owner.to_string(),
            capabilities: serde_json::to_value(capabilities).unwrap(),
            created_at: now,
            revoked_at: None,
        };
        self.keys.lock().unwrap().insert(key_hash.to_string(), key.clone());
        Ok(key)
    }

    async fn get_by_hash(&self, key_hash: &str) -> Result<Option<ApiKey>, crate::auth::keys::ApiKeyError> {
        Ok(self.keys.lock().unwrap().get(key_hash).cloned())
    }

    async fn revoke(&self, id: uuid::Uuid) -> Result<(), crate::auth::keys::ApiKeyError> {
        let mut keys = self.keys.lock().unwrap();
        if let Some(key) = keys.values_mut().find(|k| k.id == id) {
            key.revoked_at = Some(chrono::Utc::now());
        }
        Ok(())
    }

    async fn list(&self, owner: &str) -> Result<Vec<ApiKey>, crate::auth::keys::ApiKeyError> {
        Ok(self.keys.lock().unwrap().values()
            .filter(|k| k.owner == owner)
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
}
