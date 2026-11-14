//! Domain contracts for the two supported Hacienda client installation profiles.
//!
//! This crate describes topology and authorization boundaries only. It does not
//! implement encryption, a vault, a gateway, LanceDB, or a network client. Secret
//! material is represented by references to an external provider and is never a
//! field in these contracts.

use std::fmt;

use serde::{Deserialize, Serialize};
use thiserror::Error;

mod provider;

pub use provider::*;

/// A serialized configuration for one supported installation profile.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum InstallationConfig {
    /// A local Tauri client and local services/storage.
    LocalFull(LocalFullConfig),
    /// A light client connected to an authenticated, tenant-scoped on-premise gateway.
    TeamOnPremise(TeamOnPremiseConfig),
}

/// Short alias for callers that prefer the term used in product configuration files.
pub type ClientConfiguration = InstallationConfig;

/// Alias using the domain term for the profile union.
pub type InstallationProfile = InstallationConfig;

/// Alias for the LocalFull profile configuration.
pub type LocalFull = LocalFullConfig;

/// Descriptive alias for the LocalFull profile configuration.
pub type LocalFullProfile = LocalFullConfig;

/// Alias for the TeamOnPremise profile configuration.
pub type TeamOnPremise = TeamOnPremiseConfig;

/// Descriptive alias for the TeamOnPremise profile configuration.
pub type TeamOnPremiseProfile = TeamOnPremiseConfig;

/// The client surface that is allowed to use a profile.
#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ClientSurface {
    /// Full local Tauri client; the local process boundary is trusted.
    LocalTauri,
    /// Light Tauri client; all sensitive server operations cross the gateway.
    LightTauri,
}

/// The boundary at which the profile authenticates and authorizes sensitive work.
#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TrustBoundary {
    /// The local process boundary. This is only valid for LocalFull.
    LocalProcess,
    /// An authenticated gateway that enforces one tenant scope. This is only valid
    /// for TeamOnPremise.
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
        let Some(authority) = self.0.strip_prefix("http://") else {
            return false;
        };
        let host = authority.split('/').next().unwrap_or_default();
        let host = host.rsplit_once(':').map_or(host, |(host, _)| host);
        matches!(host, "localhost" | "127.0.0.1" | "[::1]")
    }

    fn is_https(&self) -> bool {
        self.0.starts_with("https://") && self.0.len() > "https://".len()
    }

    fn is_present(&self) -> bool {
        self.0.contains("://") && self.0.split("://").nth(1).is_some_and(|s| !s.is_empty())
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
pub struct EncryptedLocalVault {
    pub id: VaultId,
    pub path: StoragePath,
    pub key_reference: SecretReference,
}

/// The only valid provider route is through a redacting gateway. The direct variant
/// exists so deserialized or hand-built untrusted configuration fails closed rather
/// than silently acquiring a new route.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProviderRoute {
    /// Provider calls are made by the configured gateway after redaction.
    RedactedViaGateway { gateway: GatewayId },
    /// Explicitly forbidden: a client would reach an unredacted provider directly.
    DirectUnredacted { endpoint: ServiceEndpoint },
}

/// Gateway configuration for either profile.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum GatewayConfig {
    /// A gateway bound to the local process and local Hacienda/Xberg services.
    Local {
        id: GatewayId,
        endpoint: ServiceEndpoint,
    },
    /// An authenticated gateway that pins every operation to one tenant scope.
    AuthenticatedTenantScoped {
        id: GatewayId,
        endpoint: ServiceEndpoint,
        engine: ServiceEndpoint,
        authentication: SecretReference,
        tenant_scope: TenantScope,
    },
}

/// LanceDB topology and the authority allowed to access it.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum LanceDbTopology {
    /// An embedded local store, never addressed by a client network route.
    LocalEmbedded { path: StoragePath },
    /// Server-side control-plane access using an opaque token reference and tenant scope.
    ServerTokenOnly {
        control_plane: ServiceEndpoint,
        token_reference: SecretReference,
        tenant_scope: TenantScope,
    },
    /// Explicitly forbidden: a client reaches LanceDB directly.
    DirectClient { endpoint: ServiceEndpoint },
}

/// Storage topology of LocalFull.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LocalStorageTopology {
    pub vault: Option<EncryptedLocalVault>,
    pub lancedb: LanceDbTopology,
}

/// Storage topology of TeamOnPremise.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TeamStorageTopology {
    pub lancedb: LanceDbTopology,
}

/// Complete local profile configuration.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
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
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
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
}

impl InstallationConfig {
    /// Validate every trust-boundary and storage invariant.
    pub fn validate(&self) -> Result<(), ConfigurationError> {
        match self {
            Self::LocalFull(config) => config.validate(),
            Self::TeamOnPremise(config) => config.validate(),
        }
    }

    /// Validate a request against this profile.
    pub fn validate_rehydration(
        &self,
        request: &RehydrationRequest,
    ) -> Result<(), ConfigurationError> {
        self.validate()?;
        request.validate_for(self)
    }
}

impl LocalFullConfig {
    /// Validate the local process trust boundary and local storage topology.
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
    /// Validate the tenant gateway, token-only server storage, and server rehydration
    /// boundary.
    pub fn validate(&self) -> Result<(), ConfigurationError> {
        if self.client != ClientSurface::LightTauri
            || self.trust_boundary != TrustBoundary::AuthenticatedTenantGateway
        {
            return Err(ConfigurationError::InvalidTrustBoundary);
        }
        let profile_tenant = self
            .tenant_scope
            .as_ref()
            .ok_or(ConfigurationError::MissingTenantScope)?;
        validate_id(&profile_tenant.0)?;

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
                if tenant_scope != profile_tenant {
                    return Err(ConfigurationError::InvalidTrustBoundary);
                }
                id
            }
            Some(GatewayConfig::Local { .. }) => {
                return Err(ConfigurationError::InvalidTrustBoundary);
            }
            None => return Err(ConfigurationError::MissingGateway),
        };

        match &self.storage.lancedb {
            LanceDbTopology::ServerTokenOnly {
                control_plane,
                token_reference,
                tenant_scope,
            } => {
                validate_https(control_plane)?;
                validate_id(&token_reference.0)?;
                if tenant_scope != profile_tenant {
                    return Err(ConfigurationError::InvalidTrustBoundary);
                }
            }
            LanceDbTopology::DirectClient { .. } => {
                return Err(ConfigurationError::DirectClientToLanceDb);
            }
            LanceDbTopology::LocalEmbedded { .. } => {
                return Err(ConfigurationError::InvalidStorageTopology);
            }
        }

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

impl RehydrationRequest {
    /// Validate this request against a previously validated installation profile.
    pub fn validate_for(&self, profile: &InstallationConfig) -> Result<(), ConfigurationError> {
        if self.document_ids.iter().any(|id| id.is_empty()) {
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
                    RehydrationCapability::LocalEncryptedVault { .. } => {
                        Err(ConfigurationError::InvalidRehydrationCapability)
                    }
                    RehydrationCapability::AuditedServer { .. } => {
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
                let gateway = match config.gateway.as_ref() {
                    Some(GatewayConfig::AuthenticatedTenantScoped { id, .. }) => id,
                    _ => return Err(ConfigurationError::MissingGateway),
                };
                match &self.capability {
                    RehydrationCapability::AuditedServer {
                        gateway: request_gateway,
                        audit,
                    } if request_gateway == gateway => {
                        validate_id(&audit.0).map_err(|_| ConfigurationError::MissingAuditReference)
                    }
                    RehydrationCapability::AuditedServer { .. } => {
                        Err(ConfigurationError::InvalidRehydrationCapability)
                    }
                    RehydrationCapability::LocalEncryptedVault { .. } => {
                        Err(ConfigurationError::InvalidRehydrationCapability)
                    }
                }
            }
        }
    }
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

    fn team_config() -> InstallationConfig {
        InstallationConfig::TeamOnPremise(TeamOnPremiseConfig {
            client: ClientSurface::LightTauri,
            trust_boundary: TrustBoundary::AuthenticatedTenantGateway,
            gateway: Some(GatewayConfig::AuthenticatedTenantScoped {
                id: GatewayId("team-gateway".into()),
                endpoint: ServiceEndpoint::new("https://hacienda.example.test/gateway"),
                engine: ServiceEndpoint::new("https://hacienda.example.test/engine"),
                authentication: SecretReference("oidc:client-session".into()),
                tenant_scope: TenantScope("tenant-a".into()),
            }),
            storage: TeamStorageTopology {
                lancedb: LanceDbTopology::ServerTokenOnly {
                    control_plane: ServiceEndpoint::new("https://hacienda.example.test/lancedb"),
                    token_reference: SecretReference("control-plane:lancedb".into()),
                    tenant_scope: TenantScope("tenant-a".into()),
                },
            },
            provider_route: ProviderRoute::RedactedViaGateway {
                gateway: GatewayId("team-gateway".into()),
            },
            tenant_scope: Some(TenantScope("tenant-a".into())),
        })
    }

    #[test]
    fn accepts_valid_local_full_profile_and_local_rehydration() {
        let profile = local_config();
        profile.validate().expect("valid local profile");
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
            tenant_scope: Some(TenantScope("tenant-a".into())),
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
        let mut profile = match team_config() {
            InstallationConfig::TeamOnPremise(profile) => profile,
            InstallationConfig::LocalFull(_) => unreachable!(),
        };
        profile.tenant_scope = None;
        assert_eq!(
            profile.validate(),
            Err(ConfigurationError::MissingTenantScope)
        );

        profile.tenant_scope = Some(TenantScope("tenant-a".into()));
        profile.gateway = Some(GatewayConfig::AuthenticatedTenantScoped {
            id: GatewayId("team-gateway".into()),
            endpoint: ServiceEndpoint::new("http://hacienda.example.test/gateway"),
            engine: ServiceEndpoint::new("https://hacienda.example.test/engine"),
            authentication: SecretReference("oidc:client-session".into()),
            tenant_scope: TenantScope("tenant-a".into()),
        });
        assert_eq!(
            profile.validate(),
            Err(ConfigurationError::InsecureEndpoint)
        );
    }

    #[test]
    fn debug_does_not_include_sensitive_values() {
        let profile = local_config();
        let rendered = format!("{profile:?}");
        assert!(!rendered.contains("os-keychain:local-vault"));
        assert!(!rendered.contains("127.0.0.1"));
        assert!(rendered.contains("redacted"));
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
