//! Authentication (authn) — token validation and principal resolution.

use crate::auth::keys;
use crate::auth::{ApiKeyStore, AuthContext, AuthzError, Capability, CapabilitySet};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

/// A resolved authentication token with its associated principal and capabilities.
#[derive(Debug, Clone, Serialize, Deserialize)]
/// Token struct
pub struct Token {
    /// Opaque token identifier (for logging/auditing).
    pub id: String,
    /// The principal this token represents.
    pub principal_id: String,
    /// Human-readable name.
    pub principal_name: Option<String>,
    /// Capabilities granted by this token.
    pub capabilities: CapabilitySet,
    /// Optional expiry (Unix timestamp).
    pub expires_at: Option<u64>,
    /// Additional metadata.
    pub metadata: serde_json::Value,
}

impl Token {
    /// Create a new token.
    pub fn new(
        id: impl Into<String>,
        principal_id: impl Into<String>,
        capabilities: CapabilitySet,
    ) -> Self {
        Self {
            id: id.into(),
            principal_id: principal_id.into(),
            principal_name: None,
            capabilities,
            expires_at: None,
            metadata: serde_json::Value::Null,
        }
    }

    /// Set the expiry time (Unix timestamp).
    pub fn with_expiry(mut self, expires_at: u64) -> Self {
        self.expires_at = Some(expires_at);
        self
    }

    /// Set the principal name.
    pub fn with_name(mut self, name: impl Into<String>) -> Self {
        self.principal_name = Some(name.into());
        self
    }

    /// Set metadata.
    pub fn with_metadata(mut self, metadata: serde_json::Value) -> Self {
        self.metadata = metadata;
        self
    }

    /// Check if the token is expired.
    pub fn is_expired(&self) -> bool {
        if let Some(expires_at) = self.expires_at {
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
            now >= expires_at
        } else {
            false
        }
    }

    /// Convert to an `AuthContext` for use in request handling.
    pub fn to_auth_context(&self) -> Result<AuthContext, AuthzError> {
        if self.is_expired() {
            return Err(AuthzError::TokenExpired);
        }
        let mut ctx = AuthContext::new(&self.principal_id, self.capabilities.clone());
        ctx.principal_name = self.principal_name.clone();
        ctx.metadata = self.metadata.clone();
        Ok(ctx)
    }
}

/// Trait for resolving a token string into a `Token`.
///
/// Implementations can validate API keys, JWTs, opaque tokens against a store, etc.
#[async_trait]
/// TokenResolver trait
pub trait TokenResolver: Send + Sync {
    /// Resolve a token string to its [`Token`] representation.
    ///
    /// Returns `None` if the token is unknown or invalid (but not expired — that's checked later).
    async fn resolve(&self, token: &str) -> Result<Option<Token>, AuthzError>;
}

/// Simple in-memory token store for development and testing.
///
/// Not suitable for production — use a proper token service or database-backed implementation.
///
/// Secrets are keyed by their BLAKE3 digest rather than stored verbatim. That keeps the
/// presented secret out of the process heap for longer than the duration of a lookup, and
/// means a `HashMap` probe compares digests rather than short-circuiting on a shared prefix
/// of the secret itself.
#[derive(Debug, Default)]
/// InMemoryTokenStore struct
pub struct InMemoryTokenStore {
    /// Keyed by `blake3(secret)`; the [`Token`] carries only the loggable id.
    tokens: HashMap<[u8; 32], Token>,
}

/// Digest a presented secret into the store's key space.
fn secret_key(secret: &str) -> [u8; 32] {
    *blake3::hash(secret.as_bytes()).as_bytes()
}

impl InMemoryTokenStore {
    /// Create a new empty store.
    pub fn new() -> Self {
        Self::default()
    }

    /// Register `token`, to be resolved when a caller presents `secret`.
    ///
    /// The secret and the token's `id` are deliberately separate arguments: the id is
    /// documented as safe to log, so keying the store by it would make the audit trail a
    /// list of valid credentials.
    pub fn insert(&mut self, secret: impl AsRef<str>, token: Token) {
        self.tokens.insert(secret_key(secret.as_ref()), token);
    }

    /// Revoke the token registered under `secret`.
    pub fn remove(&mut self, secret: &str) -> Option<Token> {
        self.tokens.remove(&secret_key(secret))
    }

    /// Look up the token registered under `secret`.
    pub fn get(&self, secret: &str) -> Option<&Token> {
        self.tokens.get(&secret_key(secret))
    }

    /// List all registered tokens.
    pub fn list(&self) -> Vec<&Token> {
        self.tokens.values().collect()
    }
}

#[async_trait]
impl TokenResolver for InMemoryTokenStore {
    async fn resolve(&self, token: &str) -> Result<Option<Token>, AuthzError> {
        Ok(self.get(token).cloned())
    }
}

/// A token resolver that accepts any token and extracts capabilities from a prefix.
///
/// Format: `hcd_<capability1>,<capability2>_<random>`
/// Example: `hcd_documents:process,pii:reveal_abc123`
///
/// **Not secure** — for development only.
#[derive(Debug)]
/// DevTokenResolver struct
pub struct DevTokenResolver;

#[async_trait]
impl TokenResolver for DevTokenResolver {
    async fn resolve(&self, token: &str) -> Result<Option<Token>, AuthzError> {
        if !token.starts_with("hcd_") {
            return Ok(None);
        }

        // Split once: the random suffix is opaque and routinely contains `_`, so
        // `split('_')` rejected perfectly well-formed tokens.
        let Some((caps_str, random)) = token[4..].split_once('_') else {
            return Ok(None);
        };
        if caps_str.is_empty() || random.is_empty() {
            return Ok(None);
        }

        let capabilities: Result<CapabilitySet, _> = caps_str
            .split(',')
            .map(|s| s.parse::<Capability>())
            .collect();

        let capabilities = match capabilities {
            Ok(c) => c,
            Err(_) => return Ok(None),
        };

        let token_obj =
            Token::new(token.to_string(), "dev-user", capabilities).with_name("Development User");

        Ok(Some(token_obj))
    }
}

/// A [`TokenResolver`] backed by an [`ApiKeyStore`] — resolves bearer tokens presented as
/// issued API keys (`hacienda-core/src/auth/keys.rs`) against any `ApiKeyStore`
/// implementation (Postgres, in-memory, or any future backend).
///
/// # Why not `TokenResolverType`/`build_token_resolver`
///
/// Every other resolver in this module is fully described by [`AuthConfig`] alone —
/// `Memory` embeds its tokens, `Dev` needs nothing. This resolver instead needs a live
/// `Arc<dyn ApiKeyStore>` handle (a database pool, an in-memory store shared with the
/// rest of the process, etc.), which is not something `AuthConfig`'s serializable fields
/// can carry. Forcing it through `TokenResolverType` would mean either serializing a
/// connection string into `AuthConfig` (duplicating the store's own config) or making
/// `build_token_resolver` async and fallible in a new way just for this one variant. So
/// this follows the same pattern as `HaciendaFacade::with_stores`: the embedder
/// constructs the store, then wires it in explicitly with [`Self::new`].
///
/// # Verification
///
/// A presented token is matched by its deterministic lookup digest
/// ([`keys::lookup_key`]) — Argon2id's salted hash cannot serve as a lookup key, see the
/// `keys` module docs — and then the match is confirmed with a full Argon2id
/// [`keys::verify_key`] check against the stored hash before any capability is granted.
/// A revoked key never resolves, even if both hash checks pass.
pub struct ApiKeyTokenResolver {
    store: Arc<dyn ApiKeyStore>,
}

impl ApiKeyTokenResolver {
    /// Build a resolver over the given API key store.
    pub fn new(store: Arc<dyn ApiKeyStore>) -> Self {
        Self { store }
    }
}

impl std::fmt::Debug for ApiKeyTokenResolver {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ApiKeyTokenResolver")
            .finish_non_exhaustive()
    }
}

#[async_trait]
impl TokenResolver for ApiKeyTokenResolver {
    async fn resolve(&self, token: &str) -> Result<Option<Token>, AuthzError> {
        let lookup_hash = keys::lookup_key(token);
        let Some(api_key) = self
            .store
            .get_by_lookup_hash(&lookup_hash)
            .await
            .map_err(|e| AuthzError::InvalidToken(e.to_string()))?
        else {
            return Ok(None);
        };

        if api_key.revoked_at.is_some() {
            return Ok(None);
        }

        let verified = keys::verify_key(token, &api_key.key_hash)
            .map_err(|e| AuthzError::InvalidToken(e.to_string()))?;
        if !verified {
            return Ok(None);
        }

        // `ApiKeyStore::create` stores capabilities via `serde_json::to_value(&Vec<Capability>)`,
        // which uses `Capability`'s `#[serde(rename_all = "snake_case")]` form (e.g.
        // `"documents_process"`) — not the colon-separated `Display`/`FromStr` form
        // (`"documents:process"`) that config-file capability lists use elsewhere in this
        // module. Deserializing straight into `CapabilitySet` keeps both sides on serde's
        // representation instead of routing through the other one.
        let capabilities: CapabilitySet =
            serde_json::from_value(api_key.capabilities).map_err(|e| {
                AuthzError::InvalidToken(format!("stored capabilities are malformed: {e}"))
            })?;

        Ok(Some(Token::new(
            api_key.id.to_string(),
            api_key.owner,
            capabilities,
        )))
    }
}

/// Configuration for authentication.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
/// AuthConfig struct
pub struct AuthConfig {
    /// Enable authentication. When false, all requests are treated as unauthenticated.
    pub enabled: bool,
    /// Token resolver type. Currently supports "memory" and "dev".
    pub resolver: TokenResolverType,
    /// Optional list of static tokens (for "memory" resolver).
    pub static_tokens: Vec<StaticTokenConfig>,
}

impl Default for AuthConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            resolver: TokenResolverType::Dev,
            static_tokens: Vec::new(),
        }
    }
}

/// Type of token resolver to use.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
/// TokenResolverType enum
pub enum TokenResolverType {
    /// In-memory store with static tokens from config.
    Memory,
    /// Development resolver that parses capabilities from token prefix.
    Dev,
}

/// Static token configuration.
///
/// # The secret is write-only
///
/// `token` deserialises but does not serialise. `hacienda config show` prints the
/// effective configuration, and §6.3 of the integration design draws the line in the same
/// place for the pseudonymisation key: report the id and the resolver, never the material.
/// A `config show` that lists valid bearer tokens is a credential dump — into a terminal,
/// a support ticket, or a CI log.
///
/// `skip_serializing` rather than `skip`: loading a file must still read the secret.
#[derive(Debug, Clone, Serialize, Deserialize)]
/// StaticTokenConfig struct
pub struct StaticTokenConfig {
    /// Token identifier (for logging).
    pub id: String,
    /// The actual token string. Never serialised — see the type docs.
    #[serde(skip_serializing)]
    /// token field
    pub token: String,
    /// Principal ID.
    pub principal_id: String,
    /// Principal name.
    pub principal_name: Option<String>,
    /// Capabilities granted.
    pub capabilities: Vec<String>,
    /// Optional expiry (Unix timestamp).
    pub expires_at: Option<u64>,
}

/// Build a token resolver from `AuthConfig`.
pub fn build_token_resolver(config: &AuthConfig) -> Result<Arc<dyn TokenResolver>, AuthzError> {
    match config.resolver {
        TokenResolverType::Memory => {
            let mut store = InMemoryTokenStore::new();
            for static_token in &config.static_tokens {
                if static_token.token.is_empty() {
                    return Err(AuthzError::InvalidToken(format!(
                        "static token {} has an empty secret",
                        static_token.id
                    )));
                }

                let capabilities: CapabilitySet = static_token
                    .capabilities
                    .iter()
                    .map(|s| s.parse::<Capability>())
                    .collect::<Result<_, _>>()
                    .map_err(AuthzError::InvalidToken)?;

                let mut token = Token::new(
                    static_token.id.clone(),
                    static_token.principal_id.clone(),
                    capabilities,
                );
                if let Some(name) = &static_token.principal_name {
                    token = token.with_name(name.clone());
                }
                // `expires_at: None` means "never expires". Defaulting it to 0 made every
                // non-expiring token expire at the Unix epoch, i.e. immediately.
                if let Some(expires_at) = static_token.expires_at {
                    token = token.with_expiry(expires_at);
                }

                store.insert(&static_token.token, token);
            }
            Ok(Arc::new(store))
        }
        TokenResolverType::Dev => Ok(Arc::new(DevTokenResolver)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_expiry_check() {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();

        let token = Token::new("test", "user", CapabilitySet::empty()).with_expiry(now + 3600);
        assert!(!token.is_expired());

        let expired = Token::new("test", "user", CapabilitySet::empty()).with_expiry(now - 3600);
        assert!(expired.is_expired());

        let no_expiry = Token::new("test", "user", CapabilitySet::empty());
        assert!(!no_expiry.is_expired());
    }

    #[test]
    fn token_to_auth_context() {
        let token = Token::new(
            "test",
            "user-123",
            CapabilitySet::new([Capability::DocumentsProcess]),
        )
        .with_name("Test User");
        let ctx = token.to_auth_context().unwrap();
        assert_eq!(ctx.principal_id, "user-123");
        assert_eq!(ctx.principal_name, Some("Test User".to_string()));
        assert!(ctx.has_capability(Capability::DocumentsProcess));
    }

    #[tokio::test]
    async fn in_memory_token_store_resolve() {
        let mut store = InMemoryTokenStore::new();
        let token = Token::new(
            "token-123",
            "user-1",
            CapabilitySet::new([Capability::DocumentsProcess]),
        );
        store.insert("s3cret", token);

        let resolved = store.resolve("s3cret").await.unwrap();
        assert!(resolved.is_some());
        assert_eq!(resolved.unwrap().principal_id, "user-1");

        let not_found = store.resolve("unknown").await.unwrap();
        assert!(not_found.is_none());

        // The id is documented as safe to log, so it must not double as a credential.
        assert!(store.resolve("token-123").await.unwrap().is_none());

        assert!(store.remove("s3cret").is_some());
        assert!(store.resolve("s3cret").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn dev_token_resolver() {
        let resolver = DevTokenResolver;

        // Valid token
        let token = "hcd_documents:process,pii:reveal_abc123";
        let resolved = resolver.resolve(token).await.unwrap();
        assert!(resolved.is_some());
        let t = resolved.unwrap();
        assert!(t.capabilities.has(Capability::DocumentsProcess));
        assert!(t.capabilities.has(Capability::PiiReveal));

        // Invalid prefix
        let invalid = resolver.resolve("invalid_token").await.unwrap();
        assert!(invalid.is_none());

        // Unknown capability
        let unknown = resolver.resolve("hcd_unknown:cap_xyz").await.unwrap();
        assert!(unknown.is_none());
    }

    /// The store is keyed by `Token::id`, but `resolve` is called with the
    /// secret the client presented. Inserting under the id meant a caller
    /// authenticated by sending the *token identifier* — a value the design
    /// explicitly describes as safe to log — while the real secret never
    /// matched anything.
    #[tokio::test]
    async fn memory_resolver_matches_on_secret_not_on_id() {
        let config = AuthConfig {
            enabled: true,
            resolver: TokenResolverType::Memory,
            static_tokens: vec![StaticTokenConfig {
                id: "token-1".to_string(),
                token: "secret-token-value".to_string(),
                principal_id: "service-1".to_string(),
                principal_name: Some("Service One".to_string()),
                capabilities: vec!["documents:process".to_string(), "audit:read".to_string()],
                expires_at: None,
            }],
        };

        let resolver = build_token_resolver(&config).unwrap();

        let by_secret = resolver.resolve("secret-token-value").await.unwrap();
        assert!(
            by_secret.is_some(),
            "the presented secret must authenticate"
        );
        assert_eq!(by_secret.unwrap().principal_id, "service-1");

        let by_id = resolver.resolve("token-1").await.unwrap();
        assert!(
            by_id.is_none(),
            "the loggable token id must not authenticate"
        );
    }

    /// `expires_at: None` means "no expiry". Funnelling it through
    /// `.unwrap_or(0)` set the expiry to the Unix epoch, so every
    /// non-expiring configured token was born expired and authentication
    /// failed closed for all of them.
    #[tokio::test]
    async fn absent_expiry_means_no_expiry_not_epoch() {
        let config = AuthConfig {
            enabled: true,
            resolver: TokenResolverType::Memory,
            static_tokens: vec![StaticTokenConfig {
                id: "token-1".to_string(),
                token: "secret-token-value".to_string(),
                principal_id: "service-1".to_string(),
                principal_name: None,
                capabilities: vec!["documents:process".to_string()],
                expires_at: None,
            }],
        };

        let resolver = build_token_resolver(&config).unwrap();
        let token = resolver
            .resolve("secret-token-value")
            .await
            .unwrap()
            .unwrap();

        assert_eq!(token.expires_at, None);
        assert!(!token.is_expired());
        assert!(token.to_auth_context().is_ok());
    }

    /// An expired configured token must not authenticate.
    #[tokio::test]
    async fn expired_static_token_is_rejected() {
        let config = AuthConfig {
            enabled: true,
            resolver: TokenResolverType::Memory,
            static_tokens: vec![StaticTokenConfig {
                id: "token-1".to_string(),
                token: "secret-token-value".to_string(),
                principal_id: "service-1".to_string(),
                principal_name: None,
                capabilities: vec!["documents:process".to_string()],
                expires_at: Some(1),
            }],
        };

        let resolver = build_token_resolver(&config).unwrap();
        let token = resolver
            .resolve("secret-token-value")
            .await
            .unwrap()
            .unwrap();

        assert!(token.is_expired());
        assert!(matches!(
            token.to_auth_context(),
            Err(AuthzError::TokenExpired)
        ));
    }

    /// An unparseable capability in config is an operator error. Silently
    /// dropping the whole token, or granting it with fewer capabilities than
    /// intended, both hide the mistake.
    #[test]
    fn unknown_capability_in_config_is_an_error() {
        let config = AuthConfig {
            enabled: true,
            resolver: TokenResolverType::Memory,
            static_tokens: vec![StaticTokenConfig {
                id: "token-1".to_string(),
                token: "secret".to_string(),
                principal_id: "service-1".to_string(),
                principal_name: None,
                capabilities: vec!["documents:process".to_string(), "not:a:cap".to_string()],
                expires_at: None,
            }],
        };

        assert!(build_token_resolver(&config).is_err());
    }

    /// `hacienda config show` serialises the whole `HaciendaConfig`. If the bearer
    /// secret rides along, that command prints a working credential for every configured
    /// principal — the same failure the design forbids for the pseudonymisation key.
    #[test]
    fn should_not_serialise_the_bearer_secret_in_the_config() {
        let config = AuthConfig {
            enabled: true,
            resolver: TokenResolverType::Memory,
            static_tokens: vec![StaticTokenConfig {
                id: "token-1".to_string(),
                token: "s3cret-bearer-value".to_string(),
                principal_id: "service-1".to_string(),
                principal_name: None,
                capabilities: vec!["documents:process".to_string()],
                expires_at: None,
            }],
        };

        let json = serde_json::to_string(&config).unwrap();
        assert!(
            !json.contains("s3cret-bearer-value"),
            "config serialisation leaked a bearer token: {json}"
        );
        // The id must survive, or the operator cannot tell which token is configured.
        assert!(json.contains("token-1"));
    }

    /// The secret must still *load*: `skip_serializing`, not `skip`.
    #[test]
    fn should_still_deserialise_the_bearer_secret_from_a_config_file() {
        let toml_src = r#"
            enabled = true
            resolver = "memory"
            [[static_tokens]]
            id = "token-1"
            token = "s3cret-bearer-value"
            principal_id = "service-1"
            capabilities = ["documents:process"]
        "#;

        let config: AuthConfig = toml::from_str(toml_src).unwrap();
        assert_eq!(config.static_tokens[0].token, "s3cret-bearer-value");
    }

    /// The dev resolver splits on `_`, so a capability list followed by a
    /// random suffix containing `_` produced more than two parts and was
    /// rejected. Random suffixes routinely contain underscores.
    #[tokio::test]
    async fn dev_resolver_accepts_random_suffix_containing_underscores() {
        let resolver = DevTokenResolver;
        let resolved = resolver
            .resolve("hcd_documents:process_abc_def_123")
            .await
            .unwrap();
        assert!(resolved.is_some());
        assert!(resolved
            .unwrap()
            .capabilities
            .has(Capability::DocumentsProcess));
    }
}
