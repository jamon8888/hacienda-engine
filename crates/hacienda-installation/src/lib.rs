//! The first-class `[installation]` section — one type, one resolution, one validation.
//!
//! This crate is the shared half of the installation contract (issue #142, dossier
//! `docs/wayfinder/131-installations.md` §I1): the section a `hacienda.toml` declares and
//! the JSON `HACIENDA_INSTALLATION_CONFIG` carries are the *same* [`InstallationConfig`],
//! resolved by [`resolve_installation`] and validated by [`InstallationConfig::validate_for`].
//! There is no second, server-shaped copy of these types to keep in sync — the repository
//! already refuses that trade for every other section (see `hacienda-core/src/config.rs`).
//!
//! It lives in its own leaf crate because neither obvious home works: `hacienda-core`
//! cannot depend on the client crate (that would pull `reqwest`'s blocking client into
//! the server config path and into `wasm32` builds), and `hacienda-client-core` cannot
//! depend on `hacienda-core` (that would pull the whole Xberg stack into a trust-boundary
//! contract). `hacienda-client-core` re-exports every item here, so existing
//! `hacienda_client_core::…` paths keep working.
//!
//! This crate describes topology and authorization boundaries only. It does not
//! implement encryption, a vault, a gateway, `LanceDB`, or a network client. Secret
//! material is represented by references to an external provider and is never a
//! field in these contracts.
//!
//! # Fail-closed
//!
//! Every rule here refuses rather than degrades: an unknown key, a malformed source, two
//! sources that disagree, a direct `LanceDB` route, an unredacted provider route, a
//! tenant pinned where the server must resolve it at runtime. Errors carry no endpoint,
//! tenant, path, key, or raw source text, so they are safe to log and to return.

use std::fmt;

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// A serialized configuration for one supported installation profile.
///
/// This is the `[installation]` section of a `hacienda.{toml,yaml,json}` file *and* the
/// payload of `HACIENDA_INSTALLATION_CONFIG`: one type for both, so client and server
/// cannot drift apart. It is externally tagged, which keeps the section readable in
/// every format the config loader accepts:
///
/// ```toml
/// [installation.TeamOnPremise]
/// client = "LightTauri"
/// trust_boundary = "AuthenticatedTenantGateway"
/// # …see docs/configuration.md for the full block
/// ```
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub enum InstallationConfig {
    /// A local `Tauri` client and local services/storage.
    LocalFull(LocalFullConfig),
    /// A light client connected to an authenticated, tenant-scoped on-premise gateway.
    TeamOnPremise(TeamOnPremiseConfig),
}

/// Short alias for callers that prefer the term used in product configuration files.
pub type ClientConfiguration = InstallationConfig;

/// Alias using the domain term for the profile union.
pub type InstallationProfile = InstallationConfig;

/// Alias for the `LocalFull` profile configuration.
pub type LocalFull = LocalFullConfig;

/// Descriptive alias for the `LocalFull` profile configuration.
pub type LocalFullProfile = LocalFullConfig;

/// Alias for the `TeamOnPremise` profile configuration.
pub type TeamOnPremise = TeamOnPremiseConfig;

/// Descriptive alias for the `TeamOnPremise` profile configuration.
pub type TeamOnPremiseProfile = TeamOnPremiseConfig;

/// The client surface that is allowed to use a profile.
#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ClientSurface {
    /// Full local `Tauri` client; the local process boundary is trusted.
    LocalTauri,
    /// Light `Tauri` client; all sensitive server operations cross the gateway.
    LightTauri,
}

/// The boundary at which the profile authenticates and authorizes sensitive work.
#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TrustBoundary {
    /// The local process boundary. This is only valid for `LocalFull`.
    LocalProcess,
    /// An authenticated gateway that enforces one tenant scope. This is only valid
    /// for `TeamOnPremise`.
    AuthenticatedTenantGateway,
}

/// A non-secret service address. The value is validated as part of its profile.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServiceEndpoint(pub String);

impl ServiceEndpoint {
    /// Create an endpoint reference. Scheme and locality/security are checked when a
    /// complete profile is validated, because the allowed schemes depend on the role.
    #[must_use]
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    fn is_loopback_http(&self) -> bool {
        let Ok(url) = url::Url::parse(&self.0) else {
            return false;
        };
        url.scheme() == "http"
            && url.username().is_empty()
            && url.password().is_none()
            && matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"))
    }

    /// Accept only a parseable HTTPS URL with a host and no embedded credentials.
    /// `starts_with("https://")` alone accepted malformed values such as `"https:// "`.
    fn is_https(&self) -> bool {
        let Ok(url) = url::Url::parse(&self.0) else {
            return false;
        };
        url.scheme() == "https"
            && url.host().is_some()
            && url.username().is_empty()
            && url.password().is_none()
    }

    fn is_present(&self) -> bool {
        url::Url::parse(&self.0)
            .map(|url| url.host().is_some())
            .unwrap_or(false)
    }
}

/// A path or logical storage location. The path is not printed by `Debug`.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoragePath(pub String);

/// A reference to a secret held by an external key/token provider.
///
/// This is intentionally not key material and does not imply that cryptography is
/// implemented by this crate.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SecretReference(pub String);

/// A tenant identity supplied by the authenticated control plane.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TenantScope(pub String);

/// A stable identifier for a configured gateway.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GatewayId(pub String);

/// A stable identifier for a configured vault.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VaultId(pub String);

/// A stable identifier for an audit record or audit stream.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuditReference(pub String);

/// A local vault marker. Encryption is an external implementation concern; this type
/// only records that an encrypted local vault is required by the profile.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EncryptedLocalVault {
    pub id: VaultId,
    pub path: StoragePath,
    pub key_reference: SecretReference,
}

/// The only valid provider route is through a redacting gateway. The direct variant
/// exists so deserialized or hand-built untrusted configuration fails closed rather
/// than silently acquiring a new route.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub enum ProviderRoute {
    /// Provider calls are made by the configured gateway after redaction.
    RedactedViaGateway { gateway: GatewayId },
    /// Explicitly forbidden: a client would reach an unredacted provider directly.
    DirectUnredacted { endpoint: ServiceEndpoint },
}

/// Gateway configuration for either profile.
///
/// Unknown keys are rejected: a misspelt field in a `[installation]` block must fail
/// the load rather than silently drop the control it was meant to configure.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub enum GatewayConfig {
    /// A gateway bound to the local process and local Hacienda/Xberg services.
    Local {
        id: GatewayId,
        endpoint: ServiceEndpoint,
    },
    /// An authenticated gateway that pins every operation to one tenant scope.
    ///
    /// `tenant_scope` is `Some` in a client configuration — the client speaks for one
    /// tenant — and must be `None` in a server configuration, where the tenant is
    /// resolved from the authenticated caller on every request. A pinned tenant in a
    /// server configuration is refused, never ignored
    /// ([`ConfigurationError::TenantPinnedInServerConfig`]).
    AuthenticatedTenantScoped {
        id: GatewayId,
        endpoint: ServiceEndpoint,
        engine: ServiceEndpoint,
        authentication: SecretReference,
        tenant_scope: Option<TenantScope>,
    },
}

/// `LanceDB` topology and the authority allowed to access it.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub enum LanceDbTopology {
    /// An embedded local `LanceDB` store, never addressed by a client network route.
    LocalEmbedded { path: StoragePath },
    /// Server-side control-plane access using an opaque token reference and tenant scope.
    ///
    /// As in [`GatewayConfig::AuthenticatedTenantScoped`]: `Some` for a client, `None`
    /// for a server that resolves the tenant per request.
    ServerTokenOnly {
        control_plane: ServiceEndpoint,
        token_reference: SecretReference,
        tenant_scope: Option<TenantScope>,
    },
    /// Explicitly forbidden: a client reaches `LanceDB` directly.
    DirectClient { endpoint: ServiceEndpoint },
}

/// Storage topology of `LocalFull`.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LocalStorageTopology {
    pub vault: Option<EncryptedLocalVault>,
    pub lancedb: LanceDbTopology,
}

/// Storage topology of `TeamOnPremise`.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TeamStorageTopology {
    pub lancedb: LanceDbTopology,
}

/// Complete local profile configuration.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LocalFullConfig {
    pub client: ClientSurface,
    pub trust_boundary: TrustBoundary,
    pub hacienda: Option<ServiceEndpoint>,
    pub xberg: Option<ServiceEndpoint>,
    pub gateway: Option<GatewayConfig>,
    pub storage: LocalStorageTopology,
    pub provider_route: ProviderRoute,
}

/// Complete on-premise team profile configuration.
///
/// `tenant_scope` is the *pinned* tenant of the process that owns this configuration:
/// `Some` for a light client, `None` for the on-premise server, which resolves its
/// tenant from the authenticated caller at runtime. The profile, gateway, and storage
/// scopes must always coincide — see [`TeamOnPremiseConfig::validate_for`].
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TeamOnPremiseConfig {
    pub client: ClientSurface,
    pub trust_boundary: TrustBoundary,
    pub gateway: Option<GatewayConfig>,
    pub storage: TeamStorageTopology,
    pub provider_route: ProviderRoute,
    pub tenant_scope: Option<TenantScope>,
}

/// Capability presented with a rehydration request.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum RehydrationCapability {
    /// Local rehydration may use only the configured encrypted local vault.
    LocalEncryptedVault { vault: VaultId },
    /// Team rehydration must be performed server-side and produce an audit record.
    AuditedServer {
        gateway: GatewayId,
        audit: AuditReference,
    },
}

/// A request to rehydrate selected documents under a profile's capability boundary.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RehydrationRequest {
    pub document_ids: Vec<String>,
    pub tenant_scope: Option<TenantScope>,
    pub capability: RehydrationCapability,
}

/// Configuration and capability validation failures. Variants intentionally contain
/// no endpoint, tenant, token, path, or key values, so errors are safe to log.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Error)]
pub enum ConfigurationError {
    #[error("local profile requires an encrypted local vault")]
    MissingVault,
    #[error("profile requires a configured gateway")]
    MissingGateway,
    #[error("client-to-LanceDB access is forbidden")]
    DirectClientToLanceDb,
    #[error("direct unredacted provider routes are forbidden")]
    InsecureProviderRoute,
    #[error("endpoint is malformed or insecure for its role")]
    InsecureEndpoint,
    #[error("profile requires a tenant scope")]
    MissingTenantScope,
    #[error("profile has an invalid client surface or trust boundary")]
    InvalidTrustBoundary,
    #[error("profile has an invalid storage topology")]
    InvalidStorageTopology,
    #[error("rehydration capability is not valid for this profile")]
    InvalidRehydrationCapability,
    #[error("server-side rehydration requires an audit reference")]
    MissingAuditReference,
    #[error("configuration identifier is empty")]
    EmptyIdentifier,
    #[error("tenant scopes do not coincide across profile, gateway, and storage")]
    InconsistentTenantScope,
    #[error("a server configuration must not pin a tenant it resolves at runtime")]
    TenantPinnedInServerConfig,
}

/// The process that owns the configuration being resolved.
///
/// The section itself is shared; the *tenant binding* it may declare is not. This is the
/// answer to the dossier's open question for I1 (`docs/wayfinder/131-installations.md`
/// §10.1): `TeamOnPremise` on the server is the same section with no pinned tenant —
/// the tenant is resolved from the authenticated caller on every request, and a config
/// that pins one is refused rather than quietly overridden by the runtime scope.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum InstallationContext {
    /// The desktop client: it speaks for exactly one tenant and pins it.
    Client,
    /// The on-premise server: the tenant comes from the authenticated caller at runtime.
    Server,
}

impl InstallationConfig {
    /// Validate every trust-boundary and storage invariant for a client-owned profile.
    ///
    /// Equivalent to `validate_for(InstallationContext::Client)`, which is what every
    /// caller that has not declared a context means: the client pins its tenant.
    ///
    /// # Errors
    /// Returns the first configuration invariant that is not satisfied.
    pub fn validate(&self) -> Result<(), ConfigurationError> {
        self.validate_for(InstallationContext::Client)
    }

    /// Validate every invariant for the process that owns this configuration.
    ///
    /// Topology rules are identical in both contexts; only the tenant binding differs.
    /// `LocalFull` has no tenant at all, so its validation is context-independent.
    ///
    /// # Errors
    /// Returns the first configuration invariant that is not satisfied for `context`.
    pub fn validate_for(&self, context: InstallationContext) -> Result<(), ConfigurationError> {
        match self {
            Self::LocalFull(config) => config.validate(),
            Self::TeamOnPremise(config) => config.validate_for(context),
        }
    }

    /// Validate the topology and a coherent tenant binding without choosing one.
    ///
    /// This is the load-time check applied to a `[installation]` section as soon as it
    /// is parsed, before any process has declared whether it is the client or the
    /// server: the file may be consumed by either. A binding that is pinned in one
    /// place and absent in another is refused here
    /// ([`ConfigurationError::InconsistentTenantScope`]); which of the two coherent
    /// bindings is acceptable is then decided by [`Self::validate_for`].
    ///
    /// # Errors
    /// Returns the first configuration invariant that is not satisfied.
    pub fn validate_consistency(&self) -> Result<(), ConfigurationError> {
        match self {
            Self::LocalFull(config) => config.validate(),
            Self::TeamOnPremise(config) => config.validate_consistency(),
        }
    }

    /// Validate a request against this profile.
    ///
    /// # Errors
    /// Returns an error when the profile or the requested rehydration capability is invalid.
    /// Validate a rehydration request, choosing the process context from the profile
    /// itself: `LocalFull` and a tenant-pinned `TeamOnPremise` are client-owned; a
    /// `TeamOnPremise` profile without a pinned tenant is server-owned (the tenant is
    /// resolved from the authenticated caller, never from the request).
    ///
    /// # Errors
    /// Returns the first invalid profile invariant or an invalid request capability.
    pub fn validate_rehydration(
        &self,
        request: &RehydrationRequest,
    ) -> Result<(), ConfigurationError> {
        let context = match self {
            Self::LocalFull(_) => InstallationContext::Client,
            Self::TeamOnPremise(config) if config.tenant_scope.is_some() => {
                InstallationContext::Client
            }
            Self::TeamOnPremise(_) => InstallationContext::Server,
        };
        self.validate_rehydration_for(request, context)
    }

    /// Validate a rehydration request for the process that owns this profile.
    ///
    /// The context must be the one of the *caller* (server: tenant resolved from the
    /// authenticated principal), never derived from `request.tenant_scope`, which comes
    /// from the untrusted request. Validating a server-owned profile with the default
    /// client rules rejected every `TeamOnPremise` profile (no pinned tenant) before the
    /// request was even examined.
    ///
    /// # Errors
    /// Returns the first invalid profile invariant or an invalid request capability.
    pub fn validate_rehydration_for(
        &self,
        request: &RehydrationRequest,
        context: InstallationContext,
    ) -> Result<(), ConfigurationError> {
        self.validate_for(context)?;
        request.validate_for(self)
    }
}

impl LocalFullConfig {
    /// Validate the local process trust boundary and local storage topology.
    ///
    /// # Errors
    /// Returns an error when the trust boundary, gateway, storage, or provider route is invalid.
    pub fn validate(&self) -> Result<(), ConfigurationError> {
        if self.client != ClientSurface::LocalTauri
            || self.trust_boundary != TrustBoundary::LocalProcess
        {
            return Err(ConfigurationError::InvalidTrustBoundary);
        }

        let gateway = match self.gateway.as_ref() {
            Some(GatewayConfig::Local { id, endpoint }) => {
                validate_id(&id.0)?;
                validate_local_endpoint(endpoint)?;
                id
            }
            Some(GatewayConfig::AuthenticatedTenantScoped { .. }) => {
                return Err(ConfigurationError::InvalidTrustBoundary);
            }
            None => return Err(ConfigurationError::MissingGateway),
        };

        let vault = self
            .storage
            .vault
            .as_ref()
            .ok_or(ConfigurationError::MissingVault)?;
        validate_id(&vault.id.0)?;
        validate_non_empty(&vault.path.0)?;
        validate_id(&vault.key_reference.0)?;

        match &self.storage.lancedb {
            LanceDbTopology::LocalEmbedded { path } => validate_non_empty(&path.0)?,
            LanceDbTopology::DirectClient { .. } => {
                return Err(ConfigurationError::DirectClientToLanceDb);
            }
            LanceDbTopology::ServerTokenOnly { .. } => {
                return Err(ConfigurationError::InvalidStorageTopology);
            }
        }

        let hacienda = self
            .hacienda
            .as_ref()
            .ok_or(ConfigurationError::InvalidStorageTopology)?;
        let xberg = self
            .xberg
            .as_ref()
            .ok_or(ConfigurationError::InvalidStorageTopology)?;
        validate_local_endpoint(hacienda)?;
        validate_local_endpoint(xberg)?;

        match &self.provider_route {
            ProviderRoute::RedactedViaGateway {
                gateway: route_gateway,
            } if route_gateway == gateway => Ok(()),
            ProviderRoute::DirectUnredacted { .. } => {
                Err(ConfigurationError::InsecureProviderRoute)
            }
            ProviderRoute::RedactedViaGateway { .. } => {
                Err(ConfigurationError::InvalidTrustBoundary)
            }
        }
    }
}

impl TeamOnPremiseConfig {
    /// Validate the tenant gateway, token-only server storage, and client tenant pin.
    ///
    /// Equivalent to `validate_for(InstallationContext::Client)`.
    ///
    /// # Errors
    /// Returns an error when tenant scope, gateway, storage, or provider routing is invalid.
    pub fn validate(&self) -> Result<(), ConfigurationError> {
        self.validate_for(InstallationContext::Client)
    }

    /// Validate the profile for the process that owns it.
    ///
    /// Topology is checked first and is identical in both contexts. The tenant binding
    /// then has to match `context`: a client must pin one tenant consistently across
    /// profile, gateway, and storage; a server must pin none of the three, because the
    /// tenant is resolved from the authenticated caller on every request and a pinned
    /// one would silently shadow it.
    ///
    /// # Errors
    /// Returns an error when the topology or the tenant binding is invalid for `context`.
    pub fn validate_for(&self, context: InstallationContext) -> Result<(), ConfigurationError> {
        if context == InstallationContext::Client {
            // Checked first so "a client profile with no tenant" reports the missing
            // scope rather than whatever topology problem happens to sit next to it.
            let tenant = self
                .tenant_scope
                .as_ref()
                .ok_or(ConfigurationError::MissingTenantScope)?;
            validate_id(&tenant.0)?;
        }
        let binding = self.validate_topology()?;
        match context {
            InstallationContext::Client => {
                // Only reachable with a bound tenant: the profile scope was required
                // above, and `validate_topology` yields `None` only when all three
                // scopes are absent. Fails closed rather than asserting.
                binding.ok_or(ConfigurationError::MissingTenantScope)?;
                Ok(())
            }
            InstallationContext::Server => match binding {
                None => Ok(()),
                Some(_) => Err(ConfigurationError::TenantPinnedInServerConfig),
            },
        }
    }

    /// Validate the topology and a coherent tenant binding, whichever it is.
    ///
    /// # Errors
    /// Returns an error when the topology is invalid or the three tenant scopes do not
    /// coincide.
    pub fn validate_consistency(&self) -> Result<(), ConfigurationError> {
        self.validate_topology()?;
        Ok(())
    }

    /// Topology plus the tenant binding it implies: `None` when profile, gateway, and
    /// storage all carry no tenant (a server), `Some` when all three carry the same one
    /// (a client). Anything in between is refused.
    fn validate_topology(&self) -> Result<Option<TenantScope>, ConfigurationError> {
        if self.client != ClientSurface::LightTauri
            || self.trust_boundary != TrustBoundary::AuthenticatedTenantGateway
        {
            return Err(ConfigurationError::InvalidTrustBoundary);
        }

        let gateway = match self.gateway.as_ref() {
            Some(GatewayConfig::AuthenticatedTenantScoped {
                id,
                endpoint,
                engine,
                authentication,
                tenant_scope,
            }) => {
                validate_id(&id.0)?;
                validate_https(endpoint)?;
                validate_https(engine)?;
                validate_id(&authentication.0)?;
                tenant_scope
            }
            Some(GatewayConfig::Local { .. }) => {
                return Err(ConfigurationError::InvalidTrustBoundary);
            }
            None => return Err(ConfigurationError::MissingGateway),
        };

        let storage = match &self.storage.lancedb {
            LanceDbTopology::ServerTokenOnly {
                control_plane,
                token_reference,
                tenant_scope,
            } => {
                validate_https(control_plane)?;
                validate_id(&token_reference.0)?;
                tenant_scope
            }
            LanceDbTopology::DirectClient { .. } => {
                return Err(ConfigurationError::DirectClientToLanceDb);
            }
            LanceDbTopology::LocalEmbedded { .. } => {
                return Err(ConfigurationError::InvalidStorageTopology);
            }
        };

        // The three scopes must coincide: all absent, or all the same tenant. A scope
        // pinned in one place and absent in another is a configuration that says two
        // different things about who may read what, so it is refused rather than
        // resolved in favour of whichever one the code happens to read first.
        let binding = match (&self.tenant_scope, gateway, storage) {
            (None, None, None) => None,
            (Some(profile), Some(gateway), Some(storage))
                if profile == gateway && gateway == storage =>
            {
                validate_id(&profile.0)?;
                Some(profile.clone())
            }
            _ => return Err(ConfigurationError::InconsistentTenantScope),
        };

        let Some(GatewayConfig::AuthenticatedTenantScoped { id: gateway_id, .. }) =
            self.gateway.as_ref()
        else {
            return Err(ConfigurationError::MissingGateway);
        };
        match &self.provider_route {
            ProviderRoute::RedactedViaGateway {
                gateway: route_gateway,
            } if route_gateway == gateway_id => Ok(binding),
            ProviderRoute::DirectUnredacted { .. } => {
                Err(ConfigurationError::InsecureProviderRoute)
            }
            ProviderRoute::RedactedViaGateway { .. } => {
                Err(ConfigurationError::InvalidTrustBoundary)
            }
        }
    }
}

impl RehydrationRequest {
    /// Validate this request against a previously validated installation profile.
    ///
    /// # Errors
    /// Returns an error when document identifiers or rehydration capabilities are invalid.
    pub fn validate_for(&self, profile: &InstallationConfig) -> Result<(), ConfigurationError> {
        if self.document_ids.iter().any(String::is_empty) {
            return Err(ConfigurationError::EmptyIdentifier);
        }
        match profile {
            InstallationConfig::LocalFull(config) => {
                let vault = config
                    .storage
                    .vault
                    .as_ref()
                    .ok_or(ConfigurationError::MissingVault)?;
                if self.tenant_scope.is_some() {
                    return Err(ConfigurationError::InvalidRehydrationCapability);
                }
                match &self.capability {
                    RehydrationCapability::LocalEncryptedVault {
                        vault: request_vault,
                    } if request_vault == &vault.id => Ok(()),
                    RehydrationCapability::AuditedServer { .. }
                    | RehydrationCapability::LocalEncryptedVault { .. } => {
                        Err(ConfigurationError::InvalidRehydrationCapability)
                    }
                }
            }
            InstallationConfig::TeamOnPremise(config) => {
                let tenant = config
                    .tenant_scope
                    .as_ref()
                    .ok_or(ConfigurationError::MissingTenantScope)?;
                if self.tenant_scope.as_ref() != Some(tenant) {
                    return Err(ConfigurationError::MissingTenantScope);
                }
                let Some(GatewayConfig::AuthenticatedTenantScoped { id: gateway, .. }) =
                    config.gateway.as_ref()
                else {
                    return Err(ConfigurationError::MissingGateway);
                };
                match &self.capability {
                    RehydrationCapability::AuditedServer {
                        gateway: request_gateway,
                        audit,
                    } if request_gateway == gateway => {
                        validate_id(&audit.0).map_err(|_| ConfigurationError::MissingAuditReference)
                    }
                    RehydrationCapability::AuditedServer { .. }
                    | RehydrationCapability::LocalEncryptedVault { .. } => {
                        Err(ConfigurationError::InvalidRehydrationCapability)
                    }
                }
            }
        }
    }
}

/// Environment variable carrying an installation section as JSON.
///
/// The desktop client has always read this; the server reads it too, so a deployment
/// can inject the same section without editing a mounted file. When both sources are
/// present they must agree — see [`resolve_installation`].
pub const INSTALLATION_CONFIG_ENV: &str = "HACIENDA_INSTALLATION_CONFIG";

/// Why a resolution was refused.
///
/// Like [`ConfigurationError`], no variant carries raw source text or any configured
/// value: the message an operator sees, the log line it lands in, and any API response
/// it might become are safe to publish.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Error)]
pub enum InstallationResolutionError {
    /// No source declared an installation, and this caller requires one.
    #[error("no installation section was provided")]
    Missing,
    /// The config file and the environment describe two different installations.
    #[error("the config file and the environment declare different installations")]
    Conflict,
    /// A source exists but is not a valid installation section.
    #[error("the installation section could not be parsed")]
    Malformed,
    /// A source parsed, but the section it describes is not a valid installation.
    #[error("the installation section is invalid: {0}")]
    Invalid(ConfigurationError),
}

impl From<ConfigurationError> for InstallationResolutionError {
    fn from(error: ConfigurationError) -> Self {
        Self::Invalid(error)
    }
}

/// Resolve the installation section from both of its sources, then validate it for
/// `context`.
///
/// `section` is the `[installation]` block of a loaded config file; `environment` is
/// the raw content of [`INSTALLATION_CONFIG_ENV`]. Resolution is deliberately one
/// function used by client and server alike, so neither side can grow its own idea of
/// what "the installation" means:
///
/// * neither source present → [`InstallationResolutionError::Missing`];
/// * both present but different → [`InstallationResolutionError::Conflict`] — never
///   "the environment wins", because silently discarding a declared trust boundary is
///   the failure shape this section exists to prevent;
/// * a source that will not parse → [`InstallationResolutionError::Malformed`];
/// * a section that fails validation for `context` →
///   [`InstallationResolutionError::Invalid`].
///
/// # Errors
/// Returns the first rule above that does not hold.
pub fn resolve_installation(
    section: Option<&InstallationConfig>,
    environment: Option<&str>,
    context: InstallationContext,
) -> Result<InstallationConfig, InstallationResolutionError> {
    let injected = environment
        .map(serde_json::from_str::<InstallationConfig>)
        .transpose()
        .map_err(|_| InstallationResolutionError::Malformed)?;

    let resolved = match (section, injected) {
        (Some(file), Some(injected)) => {
            if file != &injected {
                return Err(InstallationResolutionError::Conflict);
            }
            (*file).clone()
        }
        (Some(file), None) => (*file).clone(),
        (None, Some(injected)) => injected,
        (None, None) => return Err(InstallationResolutionError::Missing),
    };

    resolved.validate_for(context)?;
    Ok(resolved)
}

/// Read [`INSTALLATION_CONFIG_ENV`] and resolve it as the only source, for `context`.
///
/// A variable that is set but not valid JSON is [`InstallationResolutionError::Malformed`],
/// never a silently absent installation: a deployment that meant to configure a profile
/// and mistyped it must not end up running as if it had configured nothing.
///
/// # Errors
/// Returns the first rule of [`resolve_installation`] that does not hold.
pub fn resolve_installation_from_environment(
    context: InstallationContext,
) -> Result<InstallationConfig, InstallationResolutionError> {
    let raw = match std::env::var(INSTALLATION_CONFIG_ENV) {
        Ok(value) => value,
        Err(std::env::VarError::NotPresent) => {
            return Err(InstallationResolutionError::Missing);
        }
        Err(std::env::VarError::NotUnicode(_)) => {
            return Err(InstallationResolutionError::Malformed);
        }
    };
    resolve_installation(None, Some(&raw), context)
}

fn validate_non_empty(value: &str) -> Result<(), ConfigurationError> {
    if value.trim().is_empty() {
        Err(ConfigurationError::EmptyIdentifier)
    } else {
        Ok(())
    }
}

fn validate_id(value: &str) -> Result<(), ConfigurationError> {
    validate_non_empty(value)
}

fn validate_local_endpoint(endpoint: &ServiceEndpoint) -> Result<(), ConfigurationError> {
    if endpoint.is_present() && endpoint.is_loopback_http() {
        Ok(())
    } else {
        Err(ConfigurationError::InsecureEndpoint)
    }
}

fn validate_https(endpoint: &ServiceEndpoint) -> Result<(), ConfigurationError> {
    if endpoint.is_present() && endpoint.is_https() {
        Ok(())
    } else {
        Err(ConfigurationError::InsecureEndpoint)
    }
}

impl fmt::Debug for InstallationConfig {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::LocalFull(_) => formatter.write_str("InstallationConfig::LocalFull(<redacted>)"),
            Self::TeamOnPremise(_) => {
                formatter.write_str("InstallationConfig::TeamOnPremise(<redacted>)")
            }
        }
    }
}

impl fmt::Debug for ServiceEndpoint {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("ServiceEndpoint(<redacted>)")
    }
}

impl fmt::Debug for StoragePath {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("StoragePath(<redacted>)")
    }
}

impl fmt::Debug for SecretReference {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("SecretReference(<redacted>)")
    }
}

macro_rules! debug_marker {
    ($($type:ty => $name:literal),+ $(,)?) => {
        $(
            impl fmt::Debug for $type {
                fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                    formatter.write_str($name)
                }
            }
        )+
    };
}

debug_marker!(
    TenantScope => "TenantScope(<redacted>)",
    GatewayId => "GatewayId(<redacted>)",
    VaultId => "VaultId(<redacted>)",
    AuditReference => "AuditReference(<redacted>)",
    EncryptedLocalVault => "EncryptedLocalVault(<redacted>)",
    ProviderRoute => "ProviderRoute(<redacted>)",
    GatewayConfig => "GatewayConfig(<redacted>)",
    LanceDbTopology => "LanceDbTopology(<redacted>)",
    LocalStorageTopology => "LocalStorageTopology(<redacted>)",
    TeamStorageTopology => "TeamStorageTopology(<redacted>)",
    LocalFullConfig => "LocalFullConfig(<redacted>)",
    TeamOnPremiseConfig => "TeamOnPremiseConfig(<redacted>)",
    RehydrationCapability => "RehydrationCapability(<redacted>)",
    RehydrationRequest => "RehydrationRequest(<redacted>)",
);

impl fmt::Debug for ClientSurface {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("ClientSurface")
            .field(&match self {
                Self::LocalTauri => "LocalTauri",
                Self::LightTauri => "LightTauri",
            })
            .finish()
    }
}

impl fmt::Debug for TrustBoundary {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("TrustBoundary")
            .field(&match self {
                Self::LocalProcess => "LocalProcess",
                Self::AuthenticatedTenantGateway => "AuthenticatedTenantGateway",
            })
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn local_config() -> InstallationConfig {
        InstallationConfig::LocalFull(LocalFullConfig {
            client: ClientSurface::LocalTauri,
            trust_boundary: TrustBoundary::LocalProcess,
            hacienda: Some(ServiceEndpoint::new("http://127.0.0.1:8001")),
            xberg: Some(ServiceEndpoint::new("http://localhost:8002")),
            gateway: Some(GatewayConfig::Local {
                id: GatewayId("local-gateway".into()),
                endpoint: ServiceEndpoint::new("http://127.0.0.1:8000"),
            }),
            storage: LocalStorageTopology {
                vault: Some(EncryptedLocalVault {
                    id: VaultId("local-vault".into()),
                    path: StoragePath("./vault".into()),
                    key_reference: SecretReference("os-keychain:local-vault".into()),
                }),
                lancedb: LanceDbTopology::LocalEmbedded {
                    path: StoragePath("./lancedb".into()),
                },
            },
            provider_route: ProviderRoute::RedactedViaGateway {
                gateway: GatewayId("local-gateway".into()),
            },
        })
    }

    #[test]
    fn local_endpoint_rejects_userinfo_with_remote_host() {
        for malicious in [
            "http://127.0.0.1:x@attacker.example/",
            "http://localhost:@attacker.example/",
        ] {
            let InstallationConfig::LocalFull(mut profile) = local_config() else {
                unreachable!();
            };
            profile.hacienda = Some(ServiceEndpoint::new(malicious));
            assert_eq!(
                InstallationConfig::LocalFull(profile).validate(),
                Err(ConfigurationError::InsecureEndpoint)
            );
        }
        assert!(ServiceEndpoint::new("http://[::1]").is_loopback_http());
    }

    fn tenant(name: &str) -> TenantScope {
        TenantScope(name.into())
    }

    /// The `TeamOnPremise` shape with an explicit binding in each of the three places a
    /// tenant can appear: profile, gateway, and storage. They must always coincide.
    fn team_config_with(
        profile: Option<TenantScope>,
        gateway: Option<TenantScope>,
        storage: Option<TenantScope>,
    ) -> TeamOnPremiseConfig {
        TeamOnPremiseConfig {
            client: ClientSurface::LightTauri,
            trust_boundary: TrustBoundary::AuthenticatedTenantGateway,
            gateway: Some(GatewayConfig::AuthenticatedTenantScoped {
                id: GatewayId("team-gateway".into()),
                endpoint: ServiceEndpoint::new("https://hacienda.example.test/gateway"),
                engine: ServiceEndpoint::new("https://hacienda.example.test/engine"),
                authentication: SecretReference("oidc:client-session".into()),
                tenant_scope: gateway,
            }),
            storage: TeamStorageTopology {
                lancedb: LanceDbTopology::ServerTokenOnly {
                    control_plane: ServiceEndpoint::new("https://hacienda.example.test/lancedb"),
                    token_reference: SecretReference("control-plane:lancedb".into()),
                    tenant_scope: storage,
                },
            },
            provider_route: ProviderRoute::RedactedViaGateway {
                gateway: GatewayId("team-gateway".into()),
            },
            tenant_scope: profile,
        }
    }

    fn team_config() -> InstallationConfig {
        InstallationConfig::TeamOnPremise(team_config_with(
            Some(tenant("tenant-a")),
            Some(tenant("tenant-a")),
            Some(tenant("tenant-a")),
        ))
    }

    /// The same installation as a server declares it: no tenant pinned anywhere, because
    /// the server resolves the tenant from the authenticated caller at runtime.
    fn team_server_config() -> InstallationConfig {
        InstallationConfig::TeamOnPremise(team_config_with(None, None, None))
    }

    #[test]
    fn accepts_valid_local_full_profile_and_local_rehydration() {
        let profile = local_config();
        profile.validate().expect("valid local profile");
        // LocalFull has no tenant at all, so it validates identically for either
        // context — the section really is shared, not two shapes behind one name.
        profile
            .validate_for(InstallationContext::Server)
            .expect("local profile is valid for a server too");
        let request = RehydrationRequest {
            document_ids: vec!["document-1".into()],
            tenant_scope: None,
            capability: RehydrationCapability::LocalEncryptedVault {
                vault: VaultId("local-vault".into()),
            },
        };
        profile
            .validate_rehydration(&request)
            .expect("valid local rehydration");
    }

    #[test]
    fn accepts_valid_team_profile_and_audited_rehydration() {
        let profile = team_config();
        profile.validate().expect("valid team profile");
        let request = RehydrationRequest {
            document_ids: vec!["document-1".into()],
            tenant_scope: Some(tenant("tenant-a")),
            capability: RehydrationCapability::AuditedServer {
                gateway: GatewayId("team-gateway".into()),
                audit: AuditReference("audit-1".into()),
            },
        };
        profile
            .validate_rehydration(&request)
            .expect("valid audited rehydration");
    }

    #[test]
    fn rejects_missing_local_vault_and_gateway() {
        let mut profile = match local_config() {
            InstallationConfig::LocalFull(profile) => profile,
            InstallationConfig::TeamOnPremise(_) => unreachable!(),
        };
        profile.storage.vault = None;
        assert_eq!(profile.validate(), Err(ConfigurationError::MissingVault));
        profile.storage.vault = local_config().local_vault_for_test();
        profile.gateway = None;
        assert_eq!(profile.validate(), Err(ConfigurationError::MissingGateway));
    }

    #[test]
    fn rejects_direct_lancedb_and_unredacted_provider_routes() {
        let mut profile = match team_config() {
            InstallationConfig::TeamOnPremise(profile) => profile,
            InstallationConfig::LocalFull(_) => unreachable!(),
        };
        profile.storage.lancedb = LanceDbTopology::DirectClient {
            endpoint: ServiceEndpoint::new("https://lancedb.example.test"),
        };
        assert_eq!(
            profile.validate(),
            Err(ConfigurationError::DirectClientToLanceDb)
        );

        profile.storage = team_config().team_storage_for_test();
        profile.provider_route = ProviderRoute::DirectUnredacted {
            endpoint: ServiceEndpoint::new("https://provider.example.test"),
        };
        assert_eq!(
            profile.validate(),
            Err(ConfigurationError::InsecureProviderRoute)
        );
    }

    #[test]
    fn rejects_missing_team_tenant_scope_and_insecure_gateway() {
        let mut profile = match team_server_config() {
            InstallationConfig::TeamOnPremise(profile) => profile,
            InstallationConfig::LocalFull(_) => unreachable!(),
        };
        // A client must pin a tenant, and it must pin the same one everywhere.
        assert_eq!(
            profile.clone().validate(),
            Err(ConfigurationError::MissingTenantScope)
        );
        assert_eq!(
            profile.clone().validate_consistency(),
            Ok(()),
            "an unbound tenant is coherent; only the client context refuses it"
        );

        // A tenant pinned in one place and absent in another says two different things
        // about who may read what, so it is refused whichever context applies.
        profile.tenant_scope = Some(tenant("tenant-a"));
        assert_eq!(
            profile.clone().validate(),
            Err(ConfigurationError::InconsistentTenantScope)
        );
        assert_eq!(
            profile.clone().validate_consistency(),
            Err(ConfigurationError::InconsistentTenantScope)
        );

        profile.gateway = Some(GatewayConfig::AuthenticatedTenantScoped {
            id: GatewayId("team-gateway".into()),
            endpoint: ServiceEndpoint::new("http://hacienda.example.test/gateway"),
            engine: ServiceEndpoint::new("https://hacienda.example.test/engine"),
            authentication: SecretReference("oidc:client-session".into()),
            tenant_scope: Some(tenant("tenant-a")),
        });
        assert_eq!(
            profile.validate(),
            Err(ConfigurationError::InsecureEndpoint)
        );
    }

    /// The dossier's open question for I1, answered in code: the server's
    /// `TeamOnPremise` section is the same type with no pinned tenant, and a server
    /// config that does pin one is refused rather than ignored.
    #[test]
    fn server_context_refuses_a_pinned_tenant_and_client_context_requires_one() {
        assert_eq!(
            team_config().validate_for(InstallationContext::Server),
            Err(ConfigurationError::TenantPinnedInServerConfig)
        );
        team_server_config()
            .validate_for(InstallationContext::Server)
            .expect("a server resolves its tenant at runtime");
        assert_eq!(
            team_server_config().validate_for(InstallationContext::Client),
            Err(ConfigurationError::MissingTenantScope)
        );
        team_config()
            .validate_for(InstallationContext::Client)
            .expect("a client pins its tenant");
    }

    #[test]
    fn debug_does_not_include_sensitive_values() {
        let profile = local_config();
        let rendered = format!("{profile:?}");
        assert!(!rendered.contains("os-keychain:local-vault"));
        assert!(!rendered.contains("127.0.0.1"));
        assert!(rendered.contains("redacted"));
    }

    #[test]
    fn resolves_the_file_section_for_a_server() {
        let resolved = resolve_installation(
            Some(&team_server_config()),
            None,
            InstallationContext::Server,
        )
        .expect("an unbound team section resolves for a server");
        assert!(matches!(resolved, InstallationConfig::TeamOnPremise(_)));
    }

    #[test]
    fn resolves_the_environment_for_a_client() {
        let serialized = serde_json::to_string(&team_config()).expect("a valid profile serializes");
        let resolved = resolve_installation(None, Some(&serialized), InstallationContext::Client)
            .expect("the client environment is a first-class source");
        assert_eq!(resolved, team_config());
    }

    #[test]
    fn refuses_when_no_source_declares_an_installation() {
        assert_eq!(
            resolve_installation(None, None, InstallationContext::Server),
            Err(InstallationResolutionError::Missing)
        );
    }

    #[test]
    fn refuses_two_sources_that_disagree() {
        // The environment used to be the client's private input; it is now a second
        // source for the same section, so a disagreement is a refusal rather than a
        // silent override of the trust boundary a file declared.
        let serialized = serde_json::to_string(&team_config()).expect("a valid profile serializes");
        assert_eq!(
            resolve_installation(
                Some(&team_server_config()),
                Some(&serialized),
                InstallationContext::Client
            ),
            Err(InstallationResolutionError::Conflict)
        );
    }

    #[test]
    fn refuses_a_malformed_source_without_echoing_it() {
        let raw = r#"{"LocalFull":{"trust_boundry":"LocalProcess"}}"#;
        let error = resolve_installation(None, Some(raw), InstallationContext::Client)
            .expect_err("an unparsable source must not resolve");
        assert_eq!(error, InstallationResolutionError::Malformed);
        assert!(!format!("{error}").contains("trust_boundry"));
        assert!(!format!("{error:?}").contains("trust_boundry"));
    }

    #[test]
    fn refuses_a_client_section_in_a_server_config_file() {
        let error = resolve_installation(Some(&team_config()), None, InstallationContext::Server)
            .expect_err("a pinned tenant has no place in a server configuration");
        assert_eq!(
            error,
            InstallationResolutionError::Invalid(ConfigurationError::TenantPinnedInServerConfig)
        );
    }

    #[test]
    fn refuses_an_unknown_key_inside_the_installation_section() {
        let raw = r#"{
            "LocalFull": {
                "client": "LocalTauri",
                "trust_boundary": "LocalProcess",
                "trust_boundry": "LocalProcess",
                "hacienda": "http://127.0.0.1:8001",
                "xberg": "http://127.0.0.1:8002",
                "gateway": {"Local": {"id": "g", "endpoint": "http://127.0.0.1:8000"}},
                "storage": {
                    "vault": {"id": "v", "path": "./vault", "key_reference": "os-keychain:v"},
                    "lancedb": {"LocalEmbedded": {"path": "./lancedb"}}
                },
                "provider_route": {"RedactedViaGateway": {"gateway": "g"}}
            }
        }"#;
        let error = serde_json::from_str::<InstallationConfig>(raw)
            .expect_err("a misspelt key must fail the load, not be discarded");
        assert!(
            error.to_string().contains("unknown field"),
            "unexpected serde error: {error}"
        );
    }

    /// Transverse exit criterion of dossier §8 (I1–I11): no configuration failure may put
    /// a raw value into a log line, a trace, an error response, or `Debug` output.
    #[test]
    fn configuration_failures_never_carry_a_raw_value() {
        let raw_values = [
            "hacienda.example.test",
            "tenant-a",
            "./lancedb",
            "./vault",
            "os-keychain:local-vault",
            "oidc:client-session",
            "control-plane:lancedb",
            "team-gateway",
        ];

        let InstallationConfig::TeamOnPremise(mut profile) = team_config() else {
            unreachable!();
        };
        // A gateway reachable over plain http is refused at validation time.
        profile.gateway = Some(GatewayConfig::AuthenticatedTenantScoped {
            id: GatewayId("team-gateway".into()),
            endpoint: ServiceEndpoint::new("http://hacienda.example.test/gateway"),
            engine: ServiceEndpoint::new("https://hacienda.example.test/engine"),
            authentication: SecretReference("oidc:client-session".into()),
            tenant_scope: Some(tenant("tenant-a")),
        });
        let section = InstallationConfig::TeamOnPremise(profile);
        let configuration_error = section.validate().expect_err("plain http is refused");
        let resolution_error = InstallationResolutionError::from(configuration_error);

        for rendered in [
            format!("{configuration_error}"),
            format!("{configuration_error:?}"),
            format!("{resolution_error}"),
            format!("{resolution_error:?}"),
            format!("{section:?}"),
        ] {
            for value in raw_values {
                assert!(
                    !rendered.contains(value),
                    "rendered error leaked {value:?}: {rendered}"
                );
            }
        }
    }

    trait TestProfileParts {
        fn local_vault_for_test(&self) -> Option<EncryptedLocalVault>;
        fn team_storage_for_test(&self) -> TeamStorageTopology;
    }

    impl TestProfileParts for InstallationConfig {
        fn local_vault_for_test(&self) -> Option<EncryptedLocalVault> {
            match self {
                Self::LocalFull(config) => config.storage.vault.clone(),
                Self::TeamOnPremise(_) => None,
            }
        }

        fn team_storage_for_test(&self) -> TeamStorageTopology {
            match self {
                Self::TeamOnPremise(config) => config.storage.clone(),
                Self::LocalFull(_) => unreachable!(),
            }
        }
    }
}
