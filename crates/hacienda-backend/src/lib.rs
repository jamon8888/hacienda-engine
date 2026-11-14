//! The narrow backend seam between Xberg extraction, Hacienda's token-only editor
//! projection, and [`hacienda_rag::RagStore`].  This crate deliberately has no provider
//! client and no vault implementation: provider input is a [`ProviderPrompt`] and
//! rehydration returns references, never plaintext.

use hacienda_client_core::{
    AuditReference, ConfigurationError, InstallationConfig, RehydrationCapability,
    RehydrationRequest, TenantScope, TokenizedText, VaultId,
};
use hacienda_core::audit::{AuditEntryInput, AuditStore, EntitySource, RedactionAction};
use hacienda_core::editor_projection::{
    EditorEnvelope, EditorProjectionOptions, SensitiveToken, project_document,
};
use hacienda_core::redaction::{KeyId, KeyResolver, Pseudonymiser};
use hacienda_core::tenancy::TenantId;
use hacienda_rag::{
    ChunkRecord, CollectionSpec, DocumentId, DocumentRecord, RagError, RagResult, RagStore,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use thiserror::Error;

/// Stable, profile-scoped addressing. A client never supplies a tenant to a store
/// operation; the backend derives this namespace from the validated profile.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CollectionScope {
    tenant: Option<String>,
}

impl CollectionScope {
    pub fn local() -> Self {
        Self { tenant: None }
    }

    pub fn tenant(tenant: impl Into<String>) -> Result<Self, BackendError> {
        let tenant = tenant.into();
        if tenant.trim().is_empty() {
            return Err(BackendError::InvalidScope);
        }
        Ok(Self {
            tenant: Some(tenant),
        })
    }

    pub fn collection_name(&self, requested: &str) -> Result<String, BackendError> {
        if requested.trim().is_empty() || requested.contains(':') {
            return Err(BackendError::InvalidCollection);
        }
        Ok(match &self.tenant {
            Some(tenant) => format!("tenant:{tenant}:{requested}"),
            None => format!("local:{requested}"),
        })
    }
}

/// An input which is safe to hand to a provider. This alias reuses the client-core
/// tokenized transport contract; callers obtain it only from
/// [`BackendDocument::provider_prompt`].
pub type ProviderPrompt = TokenizedText;

/// A token reference carried in a rehydration response. It is intentionally not a
/// `token -> value` map and therefore cannot accidentally disclose a value.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpaqueTokenReference {
    pub token_id: String,
    pub token_class: String,
    pub source: String,
}

/// Structured, viewer-facing rehydration result. `content` remains tokenized even after
/// authorization; a separate viewer may use the references with its own short-lived
/// capability. Plaintext is not representable in this type.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RehydrationResponse {
    pub document_id: String,
    pub version_id: String,
    pub tokenized_content: String,
    pub references: Vec<OpaqueTokenReference>,
    pub provenance: RehydrationProvenance,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RehydrationProvenance {
    pub scheme: String,
    pub authorized_by: String,
    pub profile: String,
    pub tenant_scope: Option<String>,
}

/// A proof-carrying Xberg extraction. The inner extraction is private so callers cannot
/// manufacture the seam from an arbitrary `ExtractedDocument`; it is only minted from a
/// Hacienda result that completed the PII-enabled redaction pipeline.
pub struct RedactedXbergDocument {
    document: xberg::ExtractedDocument,
    audit_references: Vec<AuditReference>,
}

impl RedactedXbergDocument {
    /// Take one document from a completed Hacienda processing result. A result produced
    /// without the PII pipeline is refused even when the input happened to contain no PII.
    pub fn from_hacienda_result(
        result: &hacienda_core::HaciendaResult,
        index: usize,
    ) -> Result<Self, BackendError> {
        if !result.metadata.pii_enabled || result.pii.len() != result.extraction.results.len() {
            return Err(BackendError::UnredactedDocument);
        }
        let document = result
            .extraction
            .results
            .get(index)
            .cloned()
            .ok_or(BackendError::UnredactedDocument)?;
        let audit_references = result
            .audit_entries
            .iter()
            .map(|entry| AuditReference(entry.id.clone()))
            .collect();
        Ok(Self {
            document,
            audit_references,
        })
    }

    fn audit_references(&self) -> &[AuditReference] {
        &self.audit_references
    }

    #[cfg(test)]
    fn test(document: xberg::ExtractedDocument) -> Self {
        Self {
            document,
            audit_references: Vec::new(),
        }
    }
}

/// The token-only record produced from one already-redacted Xberg document.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BackendDocument {
    editor: EditorEnvelope,
    rag_document: DocumentRecord,
    chunks: Vec<ChunkRecord>,
    token_references: Vec<OpaqueTokenReference>,
    tenant_scope: Option<String>,
    dossier_id: String,
    source_hash: String,
}

impl BackendDocument {
    pub fn editor(&self) -> &EditorEnvelope {
        &self.editor
    }
    pub fn rag_document(&self) -> &DocumentRecord {
        &self.rag_document
    }
    pub fn chunks(&self) -> &[ChunkRecord] {
        &self.chunks
    }
    pub fn tenant_scope(&self) -> Option<&str> {
        self.tenant_scope.as_deref()
    }
    pub fn source_hash(&self) -> &str {
        &self.source_hash
    }
}

/// Plaintext returned only in memory to an authorized human viewer. This type deliberately
/// has no `Serialize`, `Debug`, or `Display` implementation, and zeroizes on drop.
pub struct EphemeralPlaintext {
    value: zeroize::Zeroizing<String>,
    audit_references: Vec<AuditReference>,
}

impl EphemeralPlaintext {
    pub fn as_str(&self) -> &str {
        self.value.as_str()
    }
    pub fn audit_references(&self) -> &[AuditReference] {
        &self.audit_references
    }
}

#[derive(Debug, Error)]
pub enum VaultError {
    #[error("rehydration capability is not valid for this vault")]
    Denied,
    #[error("token is not a Hacienda pseudonym")]
    InvalidToken,
    #[error("key resolution failed")]
    KeyResolution,
    #[error("audit append failed")]
    Audit,
}

/// Local vault bridge. It accepts only the profile's explicit vault capability, resolves
/// keys through Hacienda's tenant-scoped `KeyResolver`, and records a digest-only Reveal
/// audit entry before returning the ephemeral viewer value.
pub struct VaultRehydrationAdapter {
    vault: VaultId,
    resolver: std::sync::Arc<dyn KeyResolver>,
    audit: std::sync::Arc<dyn AuditStore>,
}

impl VaultRehydrationAdapter {
    pub fn new(
        vault: VaultId,
        resolver: std::sync::Arc<dyn KeyResolver>,
        audit: std::sync::Arc<dyn AuditStore>,
    ) -> Self {
        Self {
            vault,
            resolver,
            audit,
        }
    }

    pub async fn reveal(
        &self,
        capability: &RehydrationCapability,
        tenant: &TenantScope,
        token: &str,
    ) -> Result<EphemeralPlaintext, VaultError> {
        let RehydrationCapability::LocalEncryptedVault { vault } = capability else {
            return Err(VaultError::Denied);
        };
        if vault != &self.vault {
            return Err(VaultError::Denied);
        }
        let (category, key_id) = parse_token_header(token).ok_or(VaultError::InvalidToken)?;
        let tenant = TenantId::new(tenant.0.clone());
        let key_id = KeyId::new(key_id).map_err(|_| VaultError::InvalidToken)?;
        let pseudonymiser =
            Pseudonymiser::with_active(self.resolver.as_ref(), &tenant, key_id, &[])
                .map_err(|_| VaultError::KeyResolution)?;
        let plaintext = pseudonymiser
            .reveal(token)
            .map_err(|_| VaultError::InvalidToken)?;
        let input = AuditEntryInput {
            id: uuid::Uuid::new_v4().to_string(),
            category: category.to_owned(),
            action: RedactionAction::Reveal,
            span_hash: blake3::hash(plaintext.as_bytes()).to_hex().to_string(),
            span_length: plaintext.len() as u32,
            confidence: None,
            source: EntitySource::Regex,
            pipeline_version: "hacienda-backend-vault-v1".to_owned(),
            config_hash: String::new(),
            principal: None,
            vertical: None,
            model: None,
        };
        let entry = self
            .audit
            .append(&tenant, vec![input])
            .await
            .map_err(|_| VaultError::Audit)?
            .into_iter()
            .next()
            .ok_or(VaultError::Audit)?;
        Ok(EphemeralPlaintext {
            value: zeroize::Zeroizing::new(plaintext),
            audit_references: vec![AuditReference(entry.id)],
        })
    }
}

fn parse_token_header(token: &str) -> Option<(&str, &str)> {
    let inner = token.strip_prefix('[')?.strip_suffix(']')?;
    let mut fields = inner.split(':');
    let category = fields.next()?;
    let key_id = fields.next()?;
    let body = fields.next()?;
    if category.is_empty() || key_id.is_empty() || body.is_empty() || fields.next().is_some() {
        None
    } else {
        Some((category, key_id))
    }
}

/// Profile-aware backend integration. Construction validates the client topology and
/// records the only tenant namespace this instance may address.
#[derive(Debug, Clone)]
pub struct Backend {
    profile: InstallationConfig,
    scope: CollectionScope,
}

#[derive(Debug, Error)]
pub enum BackendError {
    #[error("invalid installation profile: {0}")]
    Profile(#[from] ConfigurationError),
    #[error("invalid collection scope")]
    InvalidScope,
    #[error("invalid collection name")]
    InvalidCollection,
    #[error("TeamOnPremise LanceDB access must come from the authenticated gateway")]
    DirectLanceDbAccess,
    #[error("document is not a supported redacted extraction")]
    UnredactedDocument,
    #[error("rehydration authorization failed")]
    RehydrationDenied,
    #[error("RAG store error: {0}")]
    Rag(#[from] RagError),
    #[error("serialization failed: {0}")]
    Serialization(#[from] serde_json::Error),
}

impl Backend {
    pub fn new(profile: InstallationConfig) -> Result<Self, BackendError> {
        profile.validate()?;
        let scope = match &profile {
            InstallationConfig::LocalFull(_) => CollectionScope::local(),
            InstallationConfig::TeamOnPremise(config) => CollectionScope::tenant(
                config
                    .tenant_scope
                    .as_ref()
                    .ok_or(ConfigurationError::MissingTenantScope)?
                    .0
                    .clone(),
            )?,
        };
        Ok(Self { profile, scope })
    }

    pub fn profile(&self) -> &InstallationConfig {
        &self.profile
    }

    pub fn collection_name(&self, requested: &str) -> Result<String, BackendError> {
        self.scope.collection_name(requested)
    }

    /// Map an Xberg result after Hacienda has redacted all text and structured fields.
    /// The projection is the only source for metadata and RAG content, so raw metadata
    /// cannot bypass the editor boundary.
    pub fn project(
        &self,
        document: &RedactedXbergDocument,
        dossier_id: impl Into<String>,
        document_id: impl Into<String>,
        version_id: impl Into<String>,
    ) -> Result<BackendDocument, BackendError> {
        let dossier_id = dossier_id.into();
        let document_id = document_id.into();
        let version_id = version_id.into();
        if dossier_id.trim().is_empty()
            || document_id.trim().is_empty()
            || version_id.trim().is_empty()
        {
            return Err(BackendError::InvalidScope);
        }
        let options = EditorProjectionOptions::new(document_id.clone(), version_id.clone());
        let editor = project_document(&document.document, &options);
        let token_references = editor
            .search
            .tokens
            .iter()
            .map(|token| OpaqueTokenReference {
                token_id: token.token_id.clone(),
                token_class: token.token_class.clone(),
                source: token.source.clone(),
            })
            .collect::<Vec<_>>();
        let metadata = serde_json::to_value(&editor.metadata)?;
        let title = token_text(&editor.metadata.title);
        let full_text = document.document.content.clone();
        let source_hash = blake3::hash(full_text.as_bytes()).to_hex().to_string();
        let format_kind = editor.metadata.legal.format_kind.clone().or_else(|| {
            let mime = document.document.mime_type.as_ref();
            if mime == "application/pdf" {
                Some("pdf".into())
            } else if mime.starts_with("application/vnd.openxmlformats-officedocument.")
                || mime == "application/msword"
            {
                Some("office".into())
            } else if mime.starts_with("text/") {
                Some("text".into())
            } else {
                None
            }
        });
        let rag_document = DocumentRecord {
            external_id: Some(format!("{document_id}:{version_id}")),
            title,
            mime: Some(document.document.mime_type.to_string()),
            source_uri: None,
            full_text: full_text.clone(),
            keywords: editor
                .metadata
                .keywords
                .iter()
                .flatten()
                .map(|t| t.token_id.clone())
                .collect(),
            entities: Value::Null,
            labels: json!({
                "tenant_scope": self.tenant_scope(),
                "dossier_id": dossier_id.clone(),
                "document_id": document_id.clone(),
                "version_id": version_id.clone(),
                "metadata_token_only": metadata,
                "source_hash": source_hash.clone(),
                "editor_schema": editor.schema.clone(),
                "format_kind": format_kind,
                "audit_references": document.audit_references(),
            }),
            metadata: serde_json::to_value(&editor.provenance)?,
        };
        let chunks = vec![ChunkRecord {
            external_id: Some(format!("{document_id}:{version_id}:0")),
            ordinal: 0,
            content: full_text,
            embedding: Vec::new(),
            sparse_embedding: None,
            multi_vector: None,
            chunk_metadata: json!({"document_id": document_id, "version_id": version_id}),
        }];
        Ok(BackendDocument {
            editor,
            rag_document,
            chunks,
            token_references,
            tenant_scope: self.tenant_scope(),
            dossier_id,
            source_hash,
        })
    }

    pub fn provider_prompt(&self, document: &BackendDocument) -> ProviderPrompt {
        TokenizedText::new(
            document.rag_document.full_text.clone(),
            document.token_references.iter().map(|reference| {
                hacienda_client_core::OpaqueTokenReference::new(
                    reference.token_id.clone(),
                    reference.token_class.clone(),
                )
            }),
        )
    }

    /// Authorize a viewer operation and return only token references/provenance. The
    /// profile's capability validator is deliberately run before any token processing.
    pub fn rehydrate(
        &self,
        request: &RehydrationRequest,
        document: &BackendDocument,
        tokenized_content: &str,
    ) -> Result<RehydrationResponse, BackendError> {
        self.profile
            .validate_rehydration(request)
            .map_err(|_| BackendError::RehydrationDenied)?;
        if !request.document_ids.iter().any(|id| {
            id == document
                .rag_document
                .external_id
                .as_deref()
                .unwrap_or_default()
        }) {
            return Err(BackendError::RehydrationDenied);
        }
        // The response must be derived from the stored token-only projection. A caller
        // cannot smuggle arbitrary plaintext through the rehydration endpoint.
        if tokenized_content != document.rag_document.full_text {
            return Err(BackendError::RehydrationDenied);
        }
        Ok(RehydrationResponse {
            document_id: document.editor.document_ref.clone(),
            version_id: document.editor.version_ref.clone(),
            tokenized_content: tokenized_content.to_owned(),
            references: document.token_references.clone(),
            provenance: RehydrationProvenance {
                scheme: "hacienda-token-reference-v1".into(),
                authorized_by: match &request.capability {
                    hacienda_client_core::RehydrationCapability::LocalEncryptedVault { vault } => {
                        format!("vault:{}", vault.0)
                    }
                    hacienda_client_core::RehydrationCapability::AuditedServer {
                        gateway,
                        audit,
                    } => format!("gateway:{};audit:{}", gateway.0, audit.0),
                },
                profile: match self.profile {
                    InstallationConfig::LocalFull(_) => "LocalFull",
                    InstallationConfig::TeamOnPremise(_) => "TeamOnPremise",
                }
                .into(),
                tenant_scope: request.tenant_scope.as_ref().map(|scope| scope.0.clone()),
            },
        })
    }

    pub async fn persist_to(
        &self,
        store: &dyn RagStore,
        requested_collection: &str,
        document: &BackendDocument,
    ) -> RagResult<DocumentId> {
        if document.tenant_scope != self.tenant_scope() {
            return Err(RagError::InvalidQuery(
                "document tenant scope mismatch".into(),
            ));
        }
        let collection = self
            .collection_name(requested_collection)
            .map_err(|error| RagError::InvalidQuery(error.to_string()))?;
        store
            .ensure_collection(&CollectionSpec::new(&collection, 0))
            .await?;
        store
            .upsert_document(&collection, &document.rag_document, &document.chunks)
            .await
    }

    fn tenant_scope(&self) -> Option<String> {
        match &self.profile {
            InstallationConfig::LocalFull(_) => None,
            InstallationConfig::TeamOnPremise(config) => {
                config.tenant_scope.as_ref().map(|t| t.0.clone())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hacienda_client_core::{
        ClientSurface, EncryptedLocalVault, GatewayConfig, GatewayId, LanceDbTopology,
        LocalFullConfig, ProviderRoute, RehydrationCapability, SecretReference, ServiceEndpoint,
        StoragePath, TeamOnPremiseConfig, TenantScope, TrustBoundary, VaultId,
    };
    use hacienda_rag::{InMemoryVectorStore, RagStore};

    fn local() -> InstallationConfig {
        InstallationConfig::LocalFull(LocalFullConfig {
            client: ClientSurface::LocalTauri,
            trust_boundary: TrustBoundary::LocalProcess,
            hacienda: Some(ServiceEndpoint::new("http://127.0.0.1:8001")),
            xberg: Some(ServiceEndpoint::new("http://127.0.0.1:8002")),
            gateway: Some(GatewayConfig::Local {
                id: GatewayId("local".into()),
                endpoint: ServiceEndpoint::new("http://127.0.0.1:8000"),
            }),
            storage: hacienda_client_core::LocalStorageTopology {
                vault: Some(EncryptedLocalVault {
                    id: VaultId("vault".into()),
                    path: StoragePath("./vault".into()),
                    key_reference: SecretReference("keychain:vault".into()),
                }),
                lancedb: LanceDbTopology::LocalEmbedded {
                    path: StoragePath("./lance".into()),
                },
            },
            provider_route: ProviderRoute::RedactedViaGateway {
                gateway: GatewayId("local".into()),
            },
        })
    }

    fn team(tenant: &str) -> InstallationConfig {
        InstallationConfig::TeamOnPremise(TeamOnPremiseConfig {
            client: ClientSurface::LightTauri,
            trust_boundary: TrustBoundary::AuthenticatedTenantGateway,
            gateway: Some(GatewayConfig::AuthenticatedTenantScoped {
                id: GatewayId("team".into()),
                endpoint: ServiceEndpoint::new("https://gateway.example.test"),
                engine: ServiceEndpoint::new("https://engine.example.test"),
                authentication: SecretReference("session:opaque".into()),
                tenant_scope: TenantScope(tenant.into()),
            }),
            storage: hacienda_client_core::TeamStorageTopology {
                lancedb: LanceDbTopology::ServerTokenOnly {
                    control_plane: ServiceEndpoint::new("https://lancedb.example.test"),
                    token_reference: SecretReference("lance:opaque".into()),
                    tenant_scope: TenantScope(tenant.into()),
                },
            },
            provider_route: ProviderRoute::RedactedViaGateway {
                gateway: GatewayId("team".into()),
            },
            tenant_scope: Some(TenantScope(tenant.into())),
        })
    }

    fn document(mime: &str) -> xberg::ExtractedDocument {
        let mut document = xberg::ExtractedDocument {
            content: "Contact [PERSON:k1:IFBEGRCF]".into(),
            mime_type: mime.into(),
            ..Default::default()
        };
        document.metadata.title = Some("[TITLE:k1:IFBEG]".into());
        document
    }

    #[test]
    fn maps_text_pdf_and_office_metadata_to_token_only_records() {
        for (mime, kind) in [
            ("text/plain", "text"),
            ("application/pdf", "pdf"),
            (
                "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
                "office",
            ),
        ] {
            let backend = Backend::new(local()).unwrap();
            let record = backend
                .project(
                    &RedactedXbergDocument::test(document(mime)),
                    "case",
                    "doc",
                    "v1",
                )
                .unwrap();
            assert_eq!(record.rag_document().labels["format_kind"], kind);
            assert_eq!(
                record.rag_document().title.as_deref(),
                Some("[TITLE:k1:IFBEG]")
            );
            assert!(
                !serde_json::to_string(record.rag_document())
                    .unwrap()
                    .contains("PERSON_RAW")
            );
        }
    }

    #[test]
    fn tenant_collection_names_cannot_cross_scopes() {
        let a = Backend::new(team("tenant-a")).unwrap();
        let b = Backend::new(team("tenant-b")).unwrap();
        assert_eq!(
            a.collection_name("contracts").unwrap(),
            "tenant:tenant-a:contracts"
        );
        assert_ne!(
            a.collection_name("contracts").unwrap(),
            b.collection_name("contracts").unwrap()
        );
        assert!(a.collection_name("tenant-b:contracts").is_err());
    }

    #[test]
    fn rehydration_fails_closed_without_matching_capability() {
        let backend = Backend::new(local()).unwrap();
        let record = backend
            .project(
                &RedactedXbergDocument::test(document("text/plain")),
                "case",
                "doc",
                "v1",
            )
            .unwrap();
        let request = RehydrationRequest {
            document_ids: vec!["doc:v1".into()],
            tenant_scope: None,
            capability: RehydrationCapability::LocalEncryptedVault {
                vault: VaultId("wrong".into()),
            },
        };
        assert!(matches!(
            backend.rehydrate(&request, &record, record.rag_document().full_text.as_str()),
            Err(BackendError::RehydrationDenied)
        ));

        let authorized = RehydrationRequest {
            capability: RehydrationCapability::LocalEncryptedVault {
                vault: VaultId("vault".into()),
            },
            ..request
        };
        let response = backend
            .rehydrate(&authorized, &record, &record.rag_document().full_text)
            .unwrap();
        assert_eq!(response.references[0].token_id, "[PERSON:k1:IFBEGRCF]");
        assert!(
            backend
                .rehydrate(&authorized, &record, "raw plaintext")
                .is_err()
        );
    }

    #[tokio::test]
    async fn persistence_is_idempotent_and_provider_content_is_tokenized() {
        let backend = Backend::new(local()).unwrap();
        let record = backend
            .project(
                &RedactedXbergDocument::test(document("text/plain")),
                "case",
                "doc",
                "v1",
            )
            .unwrap();
        let store = InMemoryVectorStore::default();
        let first = backend
            .persist_to(&store, "contracts", &record)
            .await
            .unwrap();
        let second = backend
            .persist_to(&store, "contracts", &record)
            .await
            .unwrap();
        assert_eq!(first, second);
        assert_eq!(
            store
                .collection_stats("local:contracts")
                .await
                .unwrap()
                .documents,
            1
        );
        assert_eq!(
            backend.provider_prompt(&record).text,
            record.rag_document().full_text
        );
    }
}

fn token_text(tokens: &[SensitiveToken]) -> Option<String> {
    (!tokens.is_empty()).then(|| {
        tokens
            .iter()
            .map(|token| token.token_id.as_str())
            .collect::<Vec<_>>()
            .join(" ")
    })
}

#[cfg(feature = "lancedb")]
impl Backend {
    pub async fn connect_lancedb(
        profile: InstallationConfig,
        uri: &str,
    ) -> Result<(Self, hacienda_rag::backends::lancedb::LanceDbStore), BackendError> {
        let backend = Self::new(profile)?;
        if matches!(backend.profile, InstallationConfig::TeamOnPremise(_)) {
            return Err(BackendError::DirectLanceDbAccess);
        }
        let store = hacienda_rag::backends::lancedb::LanceDbStore::connect(uri).await?;
        Ok((backend, store))
    }
}
