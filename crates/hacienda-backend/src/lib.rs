//! The narrow backend seam between Xberg extraction, Hacienda's token-only editor
//! projection, and [`hacienda_rag::RagStore`].  This crate deliberately has no provider
//! client and no vault implementation: provider input is a [`ProviderPrompt`] and
//! rehydration returns references, never plaintext.

use hacienda_client_core::{
    ConfigurationError, InstallationConfig, RehydrationRequest, TokenizedText,
};
use hacienda_core::editor_projection::{
    project_document, EditorEnvelope, EditorProjectionOptions, SensitiveToken,
};
use hacienda_rag::{
    ChunkRecord, CollectionSpec, DocumentId, DocumentRecord, RagError, RagResult, RagStore,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
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

/// The token-only record produced from one already-redacted Xberg document.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BackendDocument {
    pub editor: EditorEnvelope,
    pub rag_document: DocumentRecord,
    pub chunks: Vec<ChunkRecord>,
    pub token_references: Vec<OpaqueTokenReference>,
    pub tenant_scope: Option<String>,
    pub dossier_id: String,
    pub source_hash: String,
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
        document: &xberg::ExtractedDocument,
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
        let editor = project_document(document, &options);
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
        let full_text = document.content.clone();
        let source_hash = blake3::hash(full_text.as_bytes()).to_hex().to_string();
        let format_kind = editor.metadata.legal.format_kind.clone().or_else(|| {
            let mime = document.mime_type.as_ref();
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
            mime: Some(document.mime_type.to_string()),
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
                .project(&document(mime), "case", "doc", "v1")
                .unwrap();
            assert_eq!(record.rag_document.labels["format_kind"], kind);
            assert_eq!(
                record.rag_document.title.as_deref(),
                Some("[TITLE:k1:IFBEG]")
            );
            assert!(!serde_json::to_string(&record.rag_document)
                .unwrap()
                .contains("PERSON_RAW"));
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
            .project(&document("text/plain"), "case", "doc", "v1")
            .unwrap();
        let request = RehydrationRequest {
            document_ids: vec!["doc:v1".into()],
            tenant_scope: None,
            capability: RehydrationCapability::LocalEncryptedVault {
                vault: VaultId("wrong".into()),
            },
        };
        assert!(matches!(
            backend.rehydrate(&request, &record, record.rag_document.full_text.as_str()),
            Err(BackendError::RehydrationDenied)
        ));

        let authorized = RehydrationRequest {
            capability: RehydrationCapability::LocalEncryptedVault {
                vault: VaultId("vault".into()),
            },
            ..request
        };
        let response = backend
            .rehydrate(&authorized, &record, &record.rag_document.full_text)
            .unwrap();
        assert_eq!(response.references[0].token_id, "[PERSON:k1:IFBEGRCF]");
        assert!(backend
            .rehydrate(&authorized, &record, "raw plaintext")
            .is_err());
    }

    #[tokio::test]
    async fn persistence_is_idempotent_and_provider_content_is_tokenized() {
        let backend = Backend::new(local()).unwrap();
        let record = backend
            .project(&document("text/plain"), "case", "doc", "v1")
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
            record.rag_document.full_text
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
        let store = hacienda_rag::backends::lancedb::LanceDbStore::connect(uri).await?;
        Ok((backend, store))
    }
}
