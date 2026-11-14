//! The narrow backend seam between Xberg extraction, Hacienda's token-only editor
//! projection, and [`hacienda_rag::RagStore`].  This crate deliberately has no provider
//! client and no vault implementation: provider input is a [`ProviderPrompt`] and
//! rehydration returns references, never plaintext.

use async_trait::async_trait;
use hacienda_client_core::{
    AuditReference, ConfigurationError, InstallationConfig, RehydrationCapability,
    RehydrationRequest, TenantScope, TokenizedText, VaultId,
};
use hacienda_core::audit::{AuditEntryInput, AuditStore, EntitySource, RedactionAction};
use hacienda_core::editor_projection::{
    EditorEnvelope, EditorProjectionOptions, SearchToken, SensitiveToken, project_document,
};
use hacienda_core::pii::PiiPipeline;
use hacienda_core::redaction::{KeyId, KeyResolver, Pseudonymiser, RedactionMode};
use hacienda_core::tenancy::TenantId;
use hacienda_rag::{
    ChunkRecord, CollectionSpec, DocumentId, DocumentRecord, EmbeddingError, RagError, RagResult,
    RagStore, TokenOnlyText, TokenTextEmbedder, validate_batch,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::sync::Arc;
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
#[derive(Debug, Clone, PartialEq, Serialize)]
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

    pub fn dossier_id(&self) -> &str {
        &self.dossier_id
    }
}

/// A Tiptap revision that has crossed the redaction pipeline boundary.
///
/// The fields are private and instances can only be issued by
/// [`TrustedEditorRevisionIssuer`]. The backend still validates the token shape before
/// accepting it and never treats this type as proof that a caller is allowed to write to
/// another tenant.
#[derive(Debug, Clone)]
pub struct PipelineAttestedEditorRevision {
    editor: EditorEnvelope,
    pipeline_version: String,
    pipeline_digest: String,
}

impl PipelineAttestedEditorRevision {
    fn from_pipeline_result(
        editor: EditorEnvelope,
        pipeline_version: String,
        pipeline_digest: String,
    ) -> Result<Self, BackendError> {
        validate_editor_envelope(&editor)?;
        if pipeline_version.trim().is_empty() || pipeline_digest.trim().is_empty() {
            return Err(BackendError::PipelineAttestationRequired);
        }
        Ok(Self {
            editor,
            pipeline_version,
            pipeline_digest,
        })
    }

    pub fn editor(&self) -> &EditorEnvelope {
        &self.editor
    }
}

/// Trusted issuer for editor revisions. The issuer owns the configured PII pipeline and
/// accepts only the changed ProseMirror document; it derives document/version references,
/// pipeline provenance, token marks, and audit evidence itself.
pub struct TrustedEditorRevisionIssuer {
    pipeline: Arc<PiiPipeline>,
    pipeline_version: String,
}

impl TrustedEditorRevisionIssuer {
    /// Construct an issuer from the same configured pipeline used by Hacienda processing.
    /// The pipeline version is derived from its serialized configuration, never supplied by
    /// the UI.
    pub fn new(pipeline: Arc<PiiPipeline>) -> Result<Self, BackendError> {
        if !pipeline.has_model() || pipeline.config().redaction.mode != RedactionMode::Pseudonymize
        {
            return Err(BackendError::PipelineNotReady);
        }
        let config = serde_json::to_vec(pipeline.config())?;
        let pipeline_version = format!(
            "hacienda-pii-pipeline-v1:{}",
            blake3::hash(&config).to_hex()
        );
        Ok(Self {
            pipeline,
            pipeline_version,
        })
    }

    /// Re-run the configured pipeline over changed editor text and issue an opaque revision.
    /// The caller cannot provide a version, tenant, ACL, pipeline version, or audit id.
    pub async fn issue(
        &self,
        base: &BackendDocument,
        changed_editor: &Value,
    ) -> Result<PipelineAttestedEditorRevision, BackendError> {
        if changed_editor.get("type").and_then(Value::as_str) != Some("doc") {
            return Err(BackendError::InvalidEditorRevision(
                "changed editor root is not a ProseMirror doc".into(),
            ));
        }
        let changed_text = editor_text(changed_editor)?;
        let trusted_tokens: std::collections::BTreeSet<_> = base
            .editor
            .search
            .tokens
            .iter()
            .map(|token| token.token_id.as_str())
            .collect();
        if extract_hacienda_tokens(&changed_text)
            .iter()
            .any(|token| !trusted_tokens.contains(token.as_str()))
        {
            return Err(BackendError::InvalidEditorRevision(
                "new token assertions require a separately authorized manual operation".into(),
            ));
        }
        let pipeline_result = self.pipeline.process(&changed_text).await?;
        let extracted = xberg::ExtractedDocument {
            content: pipeline_result.redacted_text.clone(),
            mime_type: base.editor.metadata.legal.mime_type.clone(),
            ..Default::default()
        };
        // Metadata is carried only from the already token-only server projection. The
        // caller has no metadata input path here.
        let options = EditorProjectionOptions::new(
            base.editor.document_ref.clone(),
            base.editor.version_ref.clone(),
        );
        let mut editor = project_document(&extracted, &options);
        editor.metadata = base.editor.metadata.clone();
        editor.provenance.extractor = "tiptap".into();
        editor.provenance.redaction_mode = Some(self.pipeline.config().redaction.mode);
        editor.provenance.redactions.clear();
        editor.search = merge_search_projection(&editor.search.tokens, &editor.metadata);
        // Exclude timing and wall-clock fields from the evidence digest so a retry of
        // identical editor text receives the same immutable version. The digest covers
        // only the redacted output and stable audit facts; detected entity text never
        // crosses this boundary.
        let stable_audit: Vec<_> = pipeline_result
            .audit_log
            .iter()
            .map(|entry| {
                (
                    &entry.category,
                    &entry.action,
                    &entry.span_hash,
                    entry.span_length,
                    entry.confidence,
                    entry.source,
                    &entry.chain_hash,
                )
            })
            .collect();
        let pipeline_digest = blake3::hash(&serde_json::to_vec(&(
            &pipeline_result.redacted_text,
            stable_audit,
        ))?)
        .to_hex()
        .to_string();
        PipelineAttestedEditorRevision::from_pipeline_result(
            editor,
            self.pipeline_version.clone(),
            pipeline_digest,
        )
    }
}

/// Result of an append-only editor save. `version_id` is allocated by the backend and
/// is never taken from the editor/client payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditorRevisionSave {
    pub document_id: String,
    pub dossier_id: String,
    pub version_id: String,
    pub rag_document_id: DocumentId,
    pub audit_references: Vec<AuditReference>,
    pub idempotent: bool,
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
    revision_lock: Arc<tokio::sync::Mutex<()>>,
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
    #[error("TeamOnPremise RAG requires an authenticated tenant gateway")]
    GatewayRequired,
    #[error("authenticated gateway tenant does not match the configured tenant")]
    TenantScopeMismatch,
    #[error("configured LocalFull LanceDB topology is not embedded")]
    InvalidLocalLanceDbTopology,
    #[error("LanceDB URI does not match the configured LocalFull storage path")]
    UnconfiguredLanceDbPath,
    #[error("document is not a supported redacted extraction")]
    UnredactedDocument,
    #[error("editor revision is not a valid Hacienda token-only projection: {0}")]
    InvalidEditorRevision(String),
    #[error("editor revision is missing a pipeline version or audit reference")]
    PipelineAttestationRequired,
    #[error("editor revision base version does not match the stored document")]
    RevisionBaseMismatch,
    #[error("editor revision base document is not present in the target collection")]
    RevisionBaseMissing,
    #[error("immutable editor version already exists with different content")]
    ImmutableVersionConflict,
    #[error("audit append failed: {0}")]
    Audit(#[from] hacienda_core::audit::AuditError),
    #[error("editor pipeline failed: {0}")]
    Pipeline(#[from] hacienda_core::pii::PiiError),
    #[error(
        "editor pipeline is not ready: a loaded NER detector and reversible pseudonymization are required"
    )]
    PipelineNotReady,
    #[error("token-only embedding failed: {0}")]
    Embedding(#[from] EmbeddingError),
    #[error("embedding collection provenance does not match the configured embedder")]
    EmbeddingProvenanceMismatch,
    #[error("rehydration authorization failed")]
    RehydrationDenied,
    #[error("RAG store error: {0}")]
    Rag(#[from] RagError),
    #[error("serialization failed: {0}")]
    Serialization(#[from] serde_json::Error),
}

/// The narrow gateway seam for TeamOnPremise RAG access.
#[async_trait]
pub trait AuthenticatedTenantGateway: Send + Sync {
    /// The tenant scope established by authentication at the gateway.
    fn tenant_scope(&self) -> &TenantScope;

    /// Return a RagStore whose operations are already restricted to `tenant_scope`.
    async fn rag_store(&self) -> RagResult<Arc<dyn RagStore>>;
}

impl Backend {
    /// Materialize the immutable token-only record written by an editor revision.
    ///
    /// This is intentionally exposed as a projection helper rather than a general
    /// document constructor: callers can only advance a record with a revision that
    /// was issued by [`TrustedEditorRevisionIssuer`] and a backend-allocated version id.
    pub fn document_for_editor_revision(
        &self,
        base: &BackendDocument,
        revision: &PipelineAttestedEditorRevision,
        version_id: &str,
    ) -> Result<BackendDocument, BackendError> {
        if version_id.trim().is_empty() {
            return Err(BackendError::InvalidEditorRevision(
                "revision version id is empty".into(),
            ));
        }
        build_editor_revision_document(self, base, revision, version_id)
    }

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
        Ok(Self {
            profile,
            scope,
            revision_lock: Arc::new(tokio::sync::Mutex::new(())),
        })
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
        // `project_document` is the single redaction boundary for extractor metadata: it
        // retains only Hacienda token references and drops every plaintext value. Keep the
        // same token-only projection in the RAG record; storing the original Xberg metadata
        // here would bypass the editor's disclosure contract.
        let metadata = serde_json::to_value(&editor.metadata)?;
        let mut rag_metadata = serde_json::to_value(&editor.provenance)?;
        if let Value::Object(fields) = &mut rag_metadata {
            fields.insert("token_only".to_owned(), metadata.clone());
        }
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
            metadata: rag_metadata,
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
        validate_backend_document(self, document)
            .map_err(|error| RagError::InvalidQuery(error.to_string()))?;
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

    /// Persist a token-only document with real dense vectors from the configured embedder.
    /// This is intentionally separate from [`Self::persist_to`], which is the full-text-only
    /// path and creates a zero-dimensional collection.
    pub async fn persist_to_with_embedder(
        &self,
        store: &dyn RagStore,
        requested_collection: &str,
        document: &BackendDocument,
        embedder: &dyn TokenTextEmbedder,
    ) -> Result<DocumentId, BackendError> {
        validate_backend_document(self, document)?;
        let embedded = embed_backend_document(document, embedder).await?;
        self.persist_embedded_to(store, requested_collection, &embedded, embedder)
            .await
    }

    async fn persist_embedded_to(
        &self,
        store: &dyn RagStore,
        requested_collection: &str,
        document: &BackendDocument,
        embedder: &dyn TokenTextEmbedder,
    ) -> Result<DocumentId, BackendError> {
        if document.tenant_scope != self.tenant_scope() {
            return Err(BackendError::InvalidScope);
        }
        let collection = self.collection_name(requested_collection)?;
        let metadata = embedder.metadata();
        let spec = metadata.collection_spec(collection.clone());
        if let Some(existing) = store.get_collection(&collection).await? {
            if existing.embedding_dim != spec.embedding_dim
                || existing.embedding_source.as_deref().is_some_and(|source| {
                    source != spec.embedding_source.as_deref().unwrap_or_default()
                })
                || (existing.embedding_source.is_some()
                    && existing.embedding_version != spec.embedding_version)
            {
                return Err(BackendError::EmbeddingProvenanceMismatch);
            }
        }
        store.ensure_collection(&spec).await?;
        let id = store
            .upsert_document(&collection, &document.rag_document, &document.chunks)
            .await?;
        store
            .set_embedding_provenance(&collection, metadata.source(), metadata.version())
            .await?;
        Ok(id)
    }

    /// Persist through an authenticated TeamOnPremise gateway. The gateway supplies
    /// the tenant-scoped store; callers cannot inject a raw LanceDB connection here.
    pub async fn persist_via_gateway<G: AuthenticatedTenantGateway>(
        &self,
        gateway: &G,
        requested_collection: &str,
        document: &BackendDocument,
    ) -> RagResult<DocumentId> {
        let expected = self.tenant_scope().ok_or_else(|| {
            RagError::InvalidQuery(
                "authenticated gateway access is only valid for TeamOnPremise".into(),
            )
        })?;
        if gateway.tenant_scope().0 != expected {
            return Err(RagError::InvalidQuery(
                BackendError::TenantScopeMismatch.to_string(),
            ));
        }
        let store = gateway.rag_store().await?;
        self.persist_to(store.as_ref(), requested_collection, document)
            .await
    }

    /// Persist vectors through an authenticated TeamOnPremise gateway. The embedder is a
    /// server-side dependency; the client still cannot inject a LanceDB connection.
    pub async fn persist_via_gateway_with_embedder<G: AuthenticatedTenantGateway>(
        &self,
        gateway: &G,
        requested_collection: &str,
        document: &BackendDocument,
        embedder: &dyn TokenTextEmbedder,
    ) -> Result<DocumentId, BackendError> {
        let expected = self
            .tenant_scope()
            .ok_or_else(|| BackendError::InvalidScope)?;
        if gateway.tenant_scope().0 != expected {
            return Err(BackendError::TenantScopeMismatch);
        }
        let store = gateway.rag_store().await?;
        self.persist_to_with_embedder(store.as_ref(), requested_collection, document, embedder)
            .await
    }

    /// Append a pipeline-attested Tiptap revision to a LocalFull store.
    ///
    /// `base` is server-side state, not a client claim: its tenant, dossier, document and
    /// version are checked against the stored record before the revision is accepted. A
    /// deterministic version id makes retries idempotent, while a different payload for
    /// the same version is rejected instead of being overwritten. The audit event is
    /// appended before the RAG write.
    pub async fn append_editor_revision(
        &self,
        audit: &dyn AuditStore,
        store: &dyn RagStore,
        requested_collection: &str,
        base: &BackendDocument,
        revision: &PipelineAttestedEditorRevision,
    ) -> Result<EditorRevisionSave, BackendError> {
        self.append_editor_revision_to_store(
            audit,
            store,
            requested_collection,
            base,
            revision,
            None,
        )
        .await
    }

    /// Append a pipeline-attested revision with real dense vectors. The full-text-only
    /// [`Self::append_editor_revision`] remains available for deployments without an
    /// embedding service.
    pub async fn append_editor_revision_with_embedder(
        &self,
        audit: &dyn AuditStore,
        store: &dyn RagStore,
        requested_collection: &str,
        base: &BackendDocument,
        revision: &PipelineAttestedEditorRevision,
        embedder: &dyn TokenTextEmbedder,
    ) -> Result<EditorRevisionSave, BackendError> {
        self.append_editor_revision_to_store(
            audit,
            store,
            requested_collection,
            base,
            revision,
            Some(embedder),
        )
        .await
    }

    /// Append a pipeline-attested Tiptap revision through an authenticated tenant gateway.
    /// The gateway, rather than any client field, establishes the tenant scope.
    pub async fn append_editor_revision_via_gateway<G: AuthenticatedTenantGateway>(
        &self,
        audit: &dyn AuditStore,
        gateway: &G,
        requested_collection: &str,
        base: &BackendDocument,
        revision: &PipelineAttestedEditorRevision,
    ) -> Result<EditorRevisionSave, BackendError> {
        let expected = self
            .tenant_scope()
            .ok_or_else(|| BackendError::InvalidScope)?;
        if gateway.tenant_scope().0 != expected {
            return Err(BackendError::TenantScopeMismatch);
        }
        let store = gateway.rag_store().await?;
        self.append_editor_revision_to_store(
            audit,
            store.as_ref(),
            requested_collection,
            base,
            revision,
            None,
        )
        .await
    }

    /// Append a revision with real vectors through an authenticated TeamOnPremise gateway.
    pub async fn append_editor_revision_via_gateway_with_embedder<G: AuthenticatedTenantGateway>(
        &self,
        audit: &dyn AuditStore,
        gateway: &G,
        requested_collection: &str,
        base: &BackendDocument,
        revision: &PipelineAttestedEditorRevision,
        embedder: &dyn TokenTextEmbedder,
    ) -> Result<EditorRevisionSave, BackendError> {
        let expected = self
            .tenant_scope()
            .ok_or_else(|| BackendError::InvalidScope)?;
        if gateway.tenant_scope().0 != expected {
            return Err(BackendError::TenantScopeMismatch);
        }
        let store = gateway.rag_store().await?;
        self.append_editor_revision_to_store(
            audit,
            store.as_ref(),
            requested_collection,
            base,
            revision,
            Some(embedder),
        )
        .await
    }

    async fn append_editor_revision_to_store(
        &self,
        audit: &dyn AuditStore,
        store: &dyn RagStore,
        requested_collection: &str,
        base: &BackendDocument,
        revision: &PipelineAttestedEditorRevision,
        embedder: Option<&dyn TokenTextEmbedder>,
    ) -> Result<EditorRevisionSave, BackendError> {
        let _guard = self.revision_lock.lock().await;
        if base.tenant_scope != self.tenant_scope()
            || base.dossier_id.trim().is_empty()
            || base.editor.document_ref.trim().is_empty()
            || base.editor.version_ref.trim().is_empty()
        {
            return Err(BackendError::RevisionBaseMismatch);
        }
        validate_backend_document(self, base)?;
        if revision.editor.document_ref != base.editor.document_ref
            || revision.editor.version_ref != base.editor.version_ref
        {
            return Err(BackendError::RevisionBaseMismatch);
        }
        validate_editor_envelope(&revision.editor)?;

        let collection = self
            .collection_name(requested_collection)
            .map_err(|error| BackendError::InvalidEditorRevision(error.to_string()))?;
        let base_id = base
            .rag_document
            .external_id
            .clone()
            .ok_or(BackendError::RevisionBaseMissing)?;
        let stored_base = store
            .get_document_chunks(&collection, &DocumentId(base_id))
            .await?
            .ok_or(BackendError::RevisionBaseMissing)?;
        let expected_base_chunks = match embedder {
            Some(embedder) => embed_backend_document(base, embedder).await?.chunks,
            None => base.chunks.clone(),
        };
        if stored_base.0 != base.rag_document || stored_base.1 != expected_base_chunks {
            return Err(BackendError::RevisionBaseMismatch);
        }

        let version_id = revision_version_id(base, revision)?;
        let candidate = build_editor_revision_document(self, base, revision, &version_id)?;
        validate_backend_document(self, &candidate)?;
        let candidate = match embedder {
            Some(embedder) => embed_backend_document(&candidate, embedder).await?,
            None => candidate,
        };
        let candidate_id = DocumentId(
            candidate
                .rag_document
                .external_id
                .clone()
                .ok_or(BackendError::ImmutableVersionConflict)?,
        );

        // The only permitted repeat is an exact replay of the same immutable version. The
        // preflight also avoids producing a second audit event for a retry.
        if let Some(existing) = store
            .get_document_chunks(&collection, &candidate_id)
            .await?
        {
            if existing.0 == candidate.rag_document && existing.1 == candidate.chunks {
                return Ok(EditorRevisionSave {
                    document_id: base.editor.document_ref.clone(),
                    dossier_id: base.dossier_id.clone(),
                    version_id,
                    rag_document_id: candidate_id,
                    audit_references: Vec::new(),
                    idempotent: true,
                });
            }
            return Err(BackendError::ImmutableVersionConflict);
        }

        let tenant = TenantId::new(self.tenant_scope().unwrap_or_else(|| "local".into()));
        let audit_entry = audit
            .append(
                &tenant,
                vec![AuditEntryInput {
                    id: uuid::Uuid::new_v4().to_string(),
                    category: "editor_revision".into(),
                    action: RedactionAction::Custom("pipeline_attested_save".into()),
                    span_hash: candidate.source_hash.clone(),
                    span_length: candidate.rag_document.full_text.chars().count() as u32,
                    confidence: None,
                    source: EntitySource::Model,
                    pipeline_version: revision.pipeline_version.clone(),
                    config_hash: revision.pipeline_digest.clone(),
                    principal: None,
                    vertical: None,
                    model: None,
                }],
            )
            .await?
            .into_iter()
            .next()
            .ok_or_else(|| {
                BackendError::Audit(hacienda_core::audit::AuditError::Backend(
                    "audit store returned no entry".into(),
                ))
            })?;

        // RagStore only exposes upsert today. The immutable, content-addressed version id
        // plus the serialized equality check above prevents ordinary overwrites; the lock
        // closes races between saves through this Backend instance. A store-level
        // create-only primitive is still required to make this atomic across processes.
        if let Some(embedder) = embedder {
            let metadata = embedder.metadata();
            let spec = metadata.collection_spec(collection.clone());
            if let Some(existing) = store.get_collection(&collection).await?
                && (existing.embedding_dim != spec.embedding_dim
                    || existing.embedding_source.as_deref().is_some_and(|source| {
                        source != spec.embedding_source.as_deref().unwrap_or_default()
                    })
                    || (existing.embedding_source.is_some()
                        && existing.embedding_version != spec.embedding_version))
            {
                return Err(BackendError::EmbeddingProvenanceMismatch);
            }
            store.ensure_collection(&spec).await?;
        } else {
            store
                .ensure_collection(&CollectionSpec::new(&collection, 0))
                .await?;
        }
        let rag_document_id = store
            .upsert_document(&collection, &candidate.rag_document, &candidate.chunks)
            .await?;
        if let Some(embedder) = embedder {
            let metadata = embedder.metadata();
            store
                .set_embedding_provenance(&collection, metadata.source(), metadata.version())
                .await?;
        }
        let audit_references = vec![AuditReference(audit_entry.id)];
        Ok(EditorRevisionSave {
            document_id: base.editor.document_ref.clone(),
            dossier_id: base.dossier_id.clone(),
            version_id,
            rag_document_id,
            audit_references,
            idempotent: false,
        })
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
    use hacienda_core::audit::InMemoryAuditStore;
    use hacienda_rag::{InMemoryVectorStore, RagStore};
    use std::sync::Arc;

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
        document.metadata.title = Some("[PERSON:k1:MFRGG2LTMVRXEZLU]".into());
        document.metadata.subject = Some("Jane Doe".into());
        document.metadata.additional.insert(
            "filename".into(),
            serde_json::json!("[FILENAME:k1:IFBEGRCF]"),
        );
        document.metadata.additional.insert(
            "owner".into(),
            serde_json::json!({
                "name": "[PERSON:k1:MFRGG2LTMVRXEZLU]",
                "email": "[EMAIL:k1:IFBEGRCF]",
            }),
        );
        document
            .metadata
            .additional
            .insert("raw_contact".into(), serde_json::json!("jane@example.com"));
        document
    }

    fn no_pii_document(mime: &str) -> xberg::ExtractedDocument {
        let mut document = xberg::ExtractedDocument {
            content: "Quarterly report".into(),
            mime_type: mime.into(),
            ..Default::default()
        };
        document.metadata.title = Some("Quarterly report".into());
        document
            .metadata
            .additional
            .insert("filename".into(), serde_json::json!("quarterly-report.pdf"));
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
                Some("[PERSON:k1:MFRGG2LTMVRXEZLU]")
            );
            let serialized = serde_json::to_string(&record).unwrap();
            assert!(serialized.contains("[FILENAME:k1:IFBEGRCF]"));
            assert!(serialized.contains("metadata_token_only"));
            assert!(!serialized.contains("PERSON_RAW"));
        }
    }

    #[test]
    fn sensitive_title_filename_and_metadata_are_token_only_across_projections() {
        let backend = Backend::new(local()).unwrap();
        let record = backend
            .project(
                &RedactedXbergDocument::test(document("text/plain")),
                "case",
                "doc",
                "v1",
            )
            .unwrap();

        assert_eq!(
            record.editor().metadata.title[0].token_id,
            "[PERSON:k1:MFRGG2LTMVRXEZLU]"
        );
        assert_eq!(
            record.editor().metadata.filename[0].token_id,
            "[FILENAME:k1:IFBEGRCF]"
        );
        assert!(
            record.editor().metadata.additional["owner"]
                .iter()
                .any(|token| token.token_id == "[PERSON:k1:MFRGG2LTMVRXEZLU]")
        );
        assert!(record.editor().metadata.subject.is_empty());
        assert!(
            !record
                .editor()
                .metadata
                .additional
                .contains_key("raw_contact")
        );

        let serialized = serde_json::to_string(record).unwrap();
        assert!(serialized.contains("[FILENAME:k1:IFBEGRCF]"));
        assert!(serialized.contains("[EMAIL:k1:IFBEGRCF]"));
        assert!(!serialized.contains("Jane Doe"));
        assert!(!serialized.contains("jane@example.com"));
        assert_eq!(
            record.rag_document().metadata["token_only"]["filename"][0]["token_id"],
            "[FILENAME:k1:IFBEGRCF]"
        );
        assert_eq!(record.chunks()[0].chunk_metadata["document_id"], "doc");
        assert_eq!(
            backend.provider_prompt(record).text,
            record.rag_document().full_text
        );
        assert!(!backend.provider_prompt(record).text.contains("Jane Doe"));
    }

    #[test]
    fn no_pii_control_does_not_fabricate_sensitive_tokens_or_leak_metadata() {
        let backend = Backend::new(local()).unwrap();
        let record = backend
            .project(
                &RedactedXbergDocument::test(no_pii_document("text/plain")),
                "case",
                "doc",
                "v1",
            )
            .unwrap();

        assert!(record.editor().search.tokens.is_empty());
        assert!(record.editor().metadata.title.is_empty());
        assert!(record.editor().metadata.filename.is_empty());
        assert!(record.editor().metadata.additional.is_empty());
        let serialized_metadata = serde_json::to_string(&record.rag_document().metadata).unwrap();
        assert!(!serialized_metadata.contains("quarterly-report.pdf"));
        assert!(!serialized_metadata.contains("Quarterly report"));
        assert_eq!(backend.provider_prompt(record).text, "Quarterly report");
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
        let (persisted, persisted_chunks) = store
            .get_document_chunks("local:contracts", &first)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            persisted.title.as_deref(),
            Some("[PERSON:k1:MFRGG2LTMVRXEZLU]")
        );
        assert_eq!(
            persisted.metadata["token_only"]["filename"][0]["token_id"],
            "[FILENAME:k1:IFBEGRCF]"
        );
        assert_eq!(persisted_chunks[0].content, "Contact [PERSON:k1:IFBEGRCF]");
        assert!(
            !serde_json::to_string(&persisted_chunks)
                .unwrap()
                .contains("Jane Doe")
        );
    }

    struct TestEmbedder {
        metadata: hacienda_rag::EmbeddingMetadata,
        calls: Arc<std::sync::atomic::AtomicUsize>,
    }

    impl TestEmbedder {
        fn new() -> Self {
            Self {
                metadata: hacienda_rag::EmbeddingMetadata::new("test:token-only", 4, 3, 8).unwrap(),
                calls: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
            }
        }
    }

    struct EmptyNer;

    #[async_trait]
    impl xberg::text::ner::NerBackend for EmptyNer {
        async fn detect(
            &self,
            _text: &str,
            _categories: &[xberg::types::entity::EntityCategory],
        ) -> xberg::Result<Vec<xberg::types::entity::Entity>> {
            Ok(Vec::new())
        }
    }

    fn ready_pipeline() -> Arc<PiiPipeline> {
        let resolver = hacienda_core::redaction::EnvKeyResolver::with_lookup(|name| match name {
            hacienda_core::redaction::ACTIVE_KEY_VAR => Some("k1".into()),
            "HACIENDA_PSEUDONYM_KEY_K1" => Some("07".repeat(hacienda_core::redaction::KEY_BYTES)),
            _ => None,
        });
        let pseudonymiser =
            Pseudonymiser::new(&resolver, &TenantId::default_tenant(), &[]).unwrap();
        let config = hacienda_core::pii::PipelineConfig {
            redaction: hacienda_core::redaction::RedactionConfig {
                mode: RedactionMode::Pseudonymize,
                ..Default::default()
            },
            ..Default::default()
        };
        let detector = hacienda_core::pii::NerDetector::new(Arc::new(EmptyNer));
        Arc::new(
            PiiPipeline::with_detector_and_pseudonymiser(
                config,
                Some(detector),
                Some(Arc::new(pseudonymiser)),
            )
            .unwrap(),
        )
    }

    #[async_trait]
    impl TokenTextEmbedder for TestEmbedder {
        fn metadata(&self) -> &hacienda_rag::EmbeddingMetadata {
            &self.metadata
        }

        async fn embed_batch(
            &self,
            inputs: &[TokenOnlyText],
        ) -> Result<Vec<Vec<f32>>, EmbeddingError> {
            self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok(inputs
                .iter()
                .enumerate()
                .map(|(index, _)| vec![index as f32, 1.0, 2.0])
                .collect())
        }
    }

    #[tokio::test]
    async fn embedded_persistence_derives_collection_provenance_and_vectors() {
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
        let embedder = TestEmbedder::new();
        let id = backend
            .persist_to_with_embedder(&store, "contracts", &record, &embedder)
            .await
            .unwrap();
        let spec = store
            .get_collection("local:contracts")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(spec.embedding_dim, 3);
        assert_eq!(spec.embedding_source.as_deref(), Some("test:token-only"));
        assert_eq!(spec.embedding_version, 4);
        let (_, chunks) = store
            .get_document_chunks("local:contracts", &id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(chunks[0].embedding, vec![0.0, 1.0, 2.0]);
        assert!(!serde_json::to_string(&chunks).unwrap().contains("Jane Doe"));
    }

    #[tokio::test]
    async fn forged_backend_document_content_is_rejected_before_embedding() {
        let backend = Backend::new(local()).unwrap();
        let record = backend
            .project(
                &RedactedXbergDocument::test(document("text/plain")),
                "case",
                "doc",
                "v1",
            )
            .unwrap();
        let mut forged = record.clone();
        forged.rag_document.full_text = "raw Jane Doe".into();
        forged.chunks[0].content = "raw Jane Doe".into();
        let store = InMemoryVectorStore::default();
        let embedder = TestEmbedder::new();
        let error = backend
            .persist_to_with_embedder(&store, "contracts", &forged, &embedder)
            .await
            .expect_err("forged projection must fail closed");
        assert!(matches!(error, BackendError::InvalidEditorRevision(_)));
        assert_eq!(embedder.calls.load(std::sync::atomic::Ordering::SeqCst), 0);
    }

    #[test]
    fn editor_issuer_fails_closed_without_model_and_reversible_mode() {
        let regex_only = Arc::new(
            PiiPipeline::with_detector(hacienda_core::pii::PipelineConfig::default(), None)
                .unwrap(),
        );
        assert!(matches!(
            TrustedEditorRevisionIssuer::new(regex_only),
            Err(BackendError::PipelineNotReady)
        ));
        let model_mask = Arc::new(
            PiiPipeline::with_detector(
                hacienda_core::pii::PipelineConfig::default(),
                Some(hacienda_core::pii::NerDetector::new(Arc::new(EmptyNer))),
            )
            .unwrap(),
        );
        assert!(matches!(
            TrustedEditorRevisionIssuer::new(model_mask),
            Err(BackendError::PipelineNotReady)
        ));
        let ready = ready_pipeline();
        assert!(ready.has_model());
        assert_eq!(ready.config().redaction.mode, RedactionMode::Pseudonymize);
        assert!(TrustedEditorRevisionIssuer::new(ready).is_ok());
    }

    #[tokio::test]
    async fn editor_revision_is_audited_before_append_and_retries_are_idempotent() {
        let backend = Backend::new(local()).unwrap();
        let base = backend
            .project(
                &RedactedXbergDocument::test(document("text/plain")),
                "case",
                "doc",
                "v1",
            )
            .unwrap();
        let store = InMemoryVectorStore::default();
        let embedder = TestEmbedder::new();
        backend
            .persist_to_with_embedder(&store, "contracts", &base, &embedder)
            .await
            .unwrap();
        let audit = InMemoryAuditStore::new("default");
        let pipeline = ready_pipeline();
        let issuer = TrustedEditorRevisionIssuer::new(pipeline).unwrap();
        let revision = issuer.issue(&base, &base.editor().editor).await.unwrap();

        let first = backend
            .append_editor_revision_with_embedder(
                &audit,
                &store,
                "contracts",
                &base,
                &revision,
                &embedder,
            )
            .await
            .unwrap();
        let second = backend
            .append_editor_revision_with_embedder(
                &audit,
                &store,
                "contracts",
                &base,
                &revision,
                &embedder,
            )
            .await
            .unwrap();
        assert!(!first.idempotent);
        assert!(second.idempotent);
        assert_eq!(first.version_id, second.version_id);
        assert_eq!(
            store
                .collection_stats("local:contracts")
                .await
                .unwrap()
                .documents,
            2
        );
        assert_eq!(
            audit.entries(&TenantId::new("local")).await.unwrap().len(),
            1
        );
        let persisted = store
            .get_document_chunks("local:contracts", &first.rag_document_id)
            .await
            .unwrap()
            .unwrap();
        let serialized = serde_json::to_string(&persisted).unwrap();
        assert!(!serialized.contains("Jane Doe"));
        assert!(serialized.contains("[PERSON:k1:IFBEGRCF]"));
        assert_eq!(persisted.0.labels["dossier_id"], "case");
        assert_eq!(persisted.0.labels["document_id"], "doc");
        assert_eq!(persisted.0.labels["version_id"], first.version_id);
        assert_eq!(persisted.1[0].embedding.len(), 3);
    }

    #[tokio::test]
    async fn trusted_issuer_redacts_changed_plaintext_before_attestation() {
        let backend = Backend::new(local()).unwrap();
        let base = backend
            .project(
                &RedactedXbergDocument::test(document("text/plain")),
                "case",
                "doc",
                "v1",
            )
            .unwrap();
        let mut editor = base.editor().clone();
        editor.editor["content"][0]["content"] = serde_json::json!([
            {"type": "text", "text": "Email bob@example.com"}
        ]);
        let pipeline = ready_pipeline();
        let issuer = TrustedEditorRevisionIssuer::new(pipeline).unwrap();
        let revision = issuer.issue(&base, &editor.editor).await.unwrap();
        let serialized = serde_json::to_string(revision.editor()).unwrap();
        assert!(!serialized.contains("bob@example.com"));
        assert!(serialized.contains("[EMAIL:"));

        editor.editor["content"][0]["content"] = serde_json::json!([
            {"type": "text", "text": "Forged [PERSON:k9:IFBEGRCF]"}
        ]);
        assert!(matches!(
            issuer.issue(&base, &editor.editor).await,
            Err(BackendError::InvalidEditorRevision(_))
        ));
    }

    struct TestGateway {
        tenant: TenantScope,
        store: Arc<dyn RagStore>,
    }

    #[async_trait]
    impl AuthenticatedTenantGateway for TestGateway {
        fn tenant_scope(&self) -> &TenantScope {
            &self.tenant
        }

        async fn rag_store(&self) -> RagResult<Arc<dyn RagStore>> {
            Ok(Arc::clone(&self.store))
        }
    }

    #[tokio::test]
    async fn cross_tenant_document_is_rejected_before_gateway_embedding() {
        let backend_a = Backend::new(team("tenant-a")).unwrap();
        let backend_b = Backend::new(team("tenant-b")).unwrap();
        let store: Arc<dyn RagStore> = Arc::new(InMemoryVectorStore::default());
        let gateway_a = TestGateway {
            tenant: TenantScope("tenant-a".into()),
            store,
        };
        let document_b = backend_b
            .project(
                &RedactedXbergDocument::test(document("text/plain")),
                "case",
                "doc-b",
                "v1",
            )
            .unwrap();
        let embedder = TestEmbedder::new();
        let error = backend_a
            .persist_via_gateway_with_embedder(&gateway_a, "contracts", &document_b, &embedder)
            .await
            .expect_err("cross-tenant document must be rejected");
        assert!(matches!(error, BackendError::InvalidScope));
        assert_eq!(embedder.calls.load(std::sync::atomic::Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn team_rag_requires_matching_authenticated_gateway_tenant() {
        let backend_a = Backend::new(team("tenant-a")).unwrap();
        let backend_b = Backend::new(team("tenant-b")).unwrap();
        let store: Arc<dyn RagStore> = Arc::new(InMemoryVectorStore::default());
        let gateway_a = TestGateway {
            tenant: TenantScope("tenant-a".into()),
            store: Arc::clone(&store),
        };
        let gateway_b = TestGateway {
            tenant: TenantScope("tenant-b".into()),
            store,
        };
        let document_a = backend_a
            .project(
                &RedactedXbergDocument::test(document("text/plain")),
                "case",
                "doc-a",
                "v1",
            )
            .unwrap();
        let document_b = backend_b
            .project(
                &RedactedXbergDocument::test(document("text/plain")),
                "case",
                "doc-b",
                "v1",
            )
            .unwrap();

        backend_a
            .persist_via_gateway(&gateway_a, "contracts", &document_a)
            .await
            .unwrap();
        assert!(
            backend_b
                .persist_via_gateway(&gateway_a, "contracts", &document_b)
                .await
                .is_err()
        );
        backend_b
            .persist_via_gateway(&gateway_b, "contracts", &document_b)
            .await
            .unwrap();

        assert_eq!(
            gateway_a
                .store
                .collection_stats("tenant:tenant-a:contracts")
                .await
                .unwrap()
                .documents,
            1
        );
        assert_eq!(
            gateway_b
                .store
                .collection_stats("tenant:tenant-b:contracts")
                .await
                .unwrap()
                .documents,
            1
        );
    }

    #[cfg(feature = "lancedb")]
    #[tokio::test]
    async fn local_lancedb_uses_configured_path_and_survives_reconnect() {
        let dir = tempfile::tempdir().unwrap();
        let mut profile = local();
        let path = dir.path().join("private-lance");
        if let InstallationConfig::LocalFull(config) = &mut profile {
            if let hacienda_client_core::LanceDbTopology::LocalEmbedded { path: configured } =
                &mut config.storage.lancedb
            {
                configured.0 = path.to_string_lossy().into_owned();
            }
        }
        let (backend, store) = Backend::connect_configured_lancedb(profile.clone())
            .await
            .unwrap();
        let record = backend
            .project(
                &RedactedXbergDocument::test(document("text/plain")),
                "case",
                "doc",
                "v1",
            )
            .unwrap();
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
        assert!(
            !serde_json::to_string(record.rag_document())
                .unwrap()
                .contains("PERSON_RAW")
        );

        drop(store);
        let (_, reopened) = Backend::connect_configured_lancedb(profile).await.unwrap();
        assert_eq!(
            reopened
                .collection_stats("local:contracts")
                .await
                .unwrap()
                .documents,
            1
        );
    }
}

fn revision_version_id(
    base: &BackendDocument,
    revision: &PipelineAttestedEditorRevision,
) -> Result<String, BackendError> {
    let payload = serde_json::to_vec(&(
        base.editor.document_ref.as_str(),
        base.editor.version_ref.as_str(),
        &revision.editor,
        &revision.pipeline_version,
        &revision.pipeline_digest,
    ))?;
    Ok(format!("v-{}", blake3::hash(&payload).to_hex()))
}

fn validate_backend_document(
    backend: &Backend,
    document: &BackendDocument,
) -> Result<(), BackendError> {
    if document.tenant_scope != backend.tenant_scope()
        || document.dossier_id.trim().is_empty()
        || document.editor.document_ref.trim().is_empty()
        || document.editor.version_ref.trim().is_empty()
    {
        return Err(BackendError::InvalidScope);
    }
    validate_editor_envelope(&document.editor)?;
    let projected_text = editor_text(&document.editor.editor)?;
    let expected_external_id = format!(
        "{}:{}",
        document.editor.document_ref, document.editor.version_ref
    );
    if projected_text != document.rag_document.full_text
        || blake3::hash(projected_text.as_bytes()).to_hex().to_string() != document.source_hash
        || document.rag_document.external_id.as_deref() != Some(expected_external_id.as_str())
    {
        return Err(BackendError::InvalidEditorRevision(
            "document content is not derived from its validated editor projection".into(),
        ));
    }
    if document.chunks.is_empty()
        || document
            .chunks
            .iter()
            .enumerate()
            .any(|(ordinal, chunk)| chunk.ordinal != ordinal as u32)
        || document
            .chunks
            .iter()
            .map(|chunk| chunk.content.as_str())
            .collect::<Vec<_>>()
            .join("\n")
            != projected_text
    {
        return Err(BackendError::InvalidEditorRevision(
            "document chunks are not derived from its editor projection".into(),
        ));
    }
    let token_only = serde_json::to_value(&document.editor.metadata)?;
    if document.rag_document.metadata["token_only"] != token_only
        || document.rag_document.labels["metadata_token_only"] != token_only
        || document.rag_document.title != token_text(&document.editor.metadata.title)
        || document.rag_document.keywords
            != document
                .editor
                .metadata
                .keywords
                .iter()
                .flatten()
                .map(|token| token.token_id.clone())
                .collect::<Vec<_>>()
    {
        return Err(BackendError::InvalidEditorRevision(
            "RAG metadata is not derived from the token-only editor projection".into(),
        ));
    }
    let references = document
        .editor
        .search
        .tokens
        .iter()
        .map(|token| OpaqueTokenReference {
            token_id: token.token_id.clone(),
            token_class: token.token_class.clone(),
            source: token.source.clone(),
        })
        .collect::<Vec<_>>();
    if references != document.token_references {
        return Err(BackendError::InvalidEditorRevision(
            "token references are not derived from the editor search projection".into(),
        ));
    }
    Ok(())
}

async fn embed_backend_document(
    document: &BackendDocument,
    embedder: &dyn TokenTextEmbedder,
) -> Result<BackendDocument, BackendError> {
    let inputs = document
        .chunks
        .iter()
        .map(|chunk| TokenOnlyText::from_token_only(chunk.content.clone()))
        .collect::<Vec<_>>();
    let vectors = embedder.embed(&inputs).await?;
    validate_batch(inputs.len(), embedder.metadata().dimension(), &vectors)?;
    let mut embedded = document.clone();
    for (chunk, vector) in embedded.chunks.iter_mut().zip(vectors) {
        chunk.embedding = vector;
    }
    Ok(embedded)
}

fn build_editor_revision_document(
    backend: &Backend,
    base: &BackendDocument,
    revision: &PipelineAttestedEditorRevision,
    version_id: &str,
) -> Result<BackendDocument, BackendError> {
    let mut editor = revision.editor.clone();
    editor.version_ref = version_id.to_owned();
    let full_text = editor_text(&editor.editor)?;
    let metadata = serde_json::to_value(&editor.metadata)?;
    let mut rag_metadata = serde_json::to_value(&editor.provenance)?;
    if let Value::Object(fields) = &mut rag_metadata {
        fields.insert("token_only".to_owned(), metadata.clone());
        fields.insert(
            "pipeline_version".to_owned(),
            Value::String(revision.pipeline_version.clone()),
        );
        fields.insert(
            "pipeline_digest".to_owned(),
            Value::String(revision.pipeline_digest.clone()),
        );
    }
    let source_hash = blake3::hash(full_text.as_bytes()).to_hex().to_string();
    let format_kind = editor.metadata.legal.format_kind.clone();
    let rag_document = DocumentRecord {
        external_id: Some(format!("{}:{}", base.editor.document_ref, version_id)),
        title: token_text(&editor.metadata.title),
        mime: Some(editor.metadata.legal.mime_type.clone()),
        source_uri: None,
        full_text: full_text.clone(),
        keywords: editor
            .metadata
            .keywords
            .iter()
            .flatten()
            .map(|token| token.token_id.clone())
            .collect(),
        entities: Value::Null,
        labels: json!({
            "tenant_scope": backend.tenant_scope(),
            "dossier_id": base.dossier_id.clone(),
            "document_id": base.editor.document_ref.clone(),
            "version_id": version_id,
            "metadata_token_only": metadata,
            "source_hash": source_hash.clone(),
            "editor_schema": editor.schema.clone(),
            "format_kind": format_kind,
            "pipeline_digest": revision.pipeline_digest.clone(),
        }),
        metadata: rag_metadata,
    };
    let chunks = full_text
        .split('\n')
        .enumerate()
        .map(|(ordinal, content)| ChunkRecord {
            external_id: Some(format!(
                "{}:{}:{}",
                base.editor.document_ref, version_id, ordinal
            )),
            ordinal: ordinal as u32,
            content: content.to_owned(),
            embedding: Vec::new(),
            sparse_embedding: None,
            multi_vector: None,
            chunk_metadata: json!({
                "tenant_scope": backend.tenant_scope(),
                "dossier_id": base.dossier_id.clone(),
                "document_id": base.editor.document_ref.clone(),
                "version_id": version_id,
                "source_hash": source_hash.clone(),
            }),
        })
        .collect::<Vec<_>>();
    let token_references = editor
        .search
        .tokens
        .iter()
        .map(|token| OpaqueTokenReference {
            token_id: token.token_id.clone(),
            token_class: token.token_class.clone(),
            source: token.source.clone(),
        })
        .collect();
    Ok(BackendDocument {
        editor,
        rag_document,
        chunks,
        token_references,
        tenant_scope: backend.tenant_scope(),
        dossier_id: base.dossier_id.clone(),
        source_hash,
    })
}

fn editor_text(editor: &Value) -> Result<String, BackendError> {
    let Some(blocks) = editor.get("content").and_then(Value::as_array) else {
        return Err(BackendError::InvalidEditorRevision(
            "editor root must contain a content array".into(),
        ));
    };
    let mut paragraphs = Vec::with_capacity(blocks.len());
    for block in blocks {
        let mut text = String::new();
        collect_text(block, &mut text)?;
        paragraphs.push(text);
    }
    Ok(paragraphs.join("\n"))
}

fn collect_text(value: &Value, output: &mut String) -> Result<(), BackendError> {
    if let Some(text) = value.get("text") {
        let Some(text) = text.as_str() else {
            return Err(BackendError::InvalidEditorRevision(
                "editor text node is not a string".into(),
            ));
        };
        output.push_str(text);
    }
    if let Some(children) = value.get("content") {
        let Some(children) = children.as_array() else {
            return Err(BackendError::InvalidEditorRevision(
                "editor content must be an array".into(),
            ));
        };
        for child in children {
            collect_text(child, output)?;
        }
    }
    Ok(())
}

fn validate_editor_envelope(editor: &EditorEnvelope) -> Result<(), BackendError> {
    if editor.schema != "hacienda.editor.v1" || editor.projection_version != 1 {
        return Err(BackendError::InvalidEditorRevision(
            "unsupported editor schema".into(),
        ));
    }
    if editor.document_ref.trim().is_empty() || editor.version_ref.trim().is_empty() {
        return Err(BackendError::InvalidEditorRevision(
            "document and version references are required".into(),
        ));
    }
    if editor.provenance.redactor != "hacienda" || !editor.provenance.offsets_are_source_only {
        return Err(BackendError::InvalidEditorRevision(
            "editor provenance is not a Hacienda source-only projection".into(),
        ));
    }
    if editor.editor.get("type").and_then(Value::as_str) != Some("doc") {
        return Err(BackendError::InvalidEditorRevision(
            "editor root is not a ProseMirror doc".into(),
        ));
    }
    let metadata = serde_json::to_value(&editor.metadata)?;
    validate_token_values(&metadata)?;
    let mut marked = std::collections::BTreeMap::<String, u32>::new();
    validate_editor_node(&editor.editor, &mut marked)?;
    collect_metadata_token_counts(&metadata, &mut marked);
    let search = editor
        .search
        .tokens
        .iter()
        .map(|token| {
            validate_token_id(&token.token_id)?;
            if token.source == "manual" || token.mode == "manual-mask" {
                return Err(BackendError::InvalidEditorRevision(
                    "manual editor marks are not pipeline-attested".into(),
                ));
            }
            Ok((token.token_id.clone(), token.occurrences))
        })
        .collect::<Result<std::collections::BTreeMap<_, _>, BackendError>>()?;
    if marked != search {
        return Err(BackendError::InvalidEditorRevision(
            "search projection does not match editor token marks".into(),
        ));
    }
    Ok(())
}

fn validate_token_values(value: &Value) -> Result<(), BackendError> {
    match value {
        Value::Array(values) => values.iter().try_for_each(validate_token_values),
        Value::Object(fields) => {
            if fields.contains_key("tokenId") || fields.contains_key("token_id") {
                let id = fields
                    .get("tokenId")
                    .or_else(|| fields.get("token_id"))
                    .and_then(Value::as_str)
                    .ok_or_else(|| {
                        BackendError::InvalidEditorRevision(
                            "token metadata id is not a string".into(),
                        )
                    })?;
                validate_token_id(id)?;
                let source = fields
                    .get("source")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let mode = fields
                    .get("mode")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                if source == "manual" || mode == "manual-mask" {
                    return Err(BackendError::InvalidEditorRevision(
                        "manual token metadata is not pipeline-attested".into(),
                    ));
                }
            }
            fields.values().try_for_each(validate_token_values)
        }
        _ => Ok(()),
    }
}

fn collect_metadata_token_counts(
    value: &Value,
    counts: &mut std::collections::BTreeMap<String, u32>,
) {
    match value {
        Value::Array(values) => {
            for value in values {
                collect_metadata_token_counts(value, counts);
            }
        }
        Value::Object(fields) => {
            if let Some(token_id) = fields
                .get("tokenId")
                .or_else(|| fields.get("token_id"))
                .and_then(Value::as_str)
            {
                *counts.entry(token_id.to_owned()).or_default() += 1;
            }
            for value in fields.values() {
                collect_metadata_token_counts(value, counts);
            }
        }
        _ => {}
    }
}

fn merge_search_projection(
    body_tokens: &[SearchToken],
    metadata: &hacienda_core::editor_projection::TokenOnlyMetadata,
) -> hacienda_core::editor_projection::SearchProjection {
    let mut merged = std::collections::BTreeMap::<String, SearchToken>::new();
    for token in body_tokens {
        merged.insert(token.token_id.clone(), token.clone());
    }
    let metadata_value = serde_json::to_value(metadata).unwrap_or(Value::Null);
    collect_metadata_search_tokens(&metadata_value, &mut merged);
    hacienda_core::editor_projection::SearchProjection {
        tokens: merged.into_values().collect(),
    }
}

fn collect_metadata_search_tokens(
    value: &Value,
    tokens: &mut std::collections::BTreeMap<String, SearchToken>,
) {
    match value {
        Value::Array(values) => {
            for value in values {
                collect_metadata_search_tokens(value, tokens);
            }
        }
        Value::Object(fields) => {
            let Some(token_id) = fields
                .get("tokenId")
                .or_else(|| fields.get("token_id"))
                .and_then(Value::as_str)
            else {
                for value in fields.values() {
                    collect_metadata_search_tokens(value, tokens);
                }
                return;
            };
            let token = SearchToken {
                token_class: fields
                    .get("tokenClass")
                    .or_else(|| fields.get("token_class"))
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned(),
                token_id: token_id.to_owned(),
                mode: fields
                    .get("mode")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned(),
                source: fields
                    .get("source")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned(),
                disclosure_state: fields
                    .get("disclosureState")
                    .or_else(|| fields.get("disclosure_state"))
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned(),
                occurrences: 1,
            };
            if let Some(existing) = tokens.get_mut(token_id) {
                existing.occurrences += 1;
            } else {
                tokens.insert(token_id.to_owned(), token);
            }
        }
        _ => {}
    }
}

fn validate_editor_node(
    value: &Value,
    marked: &mut std::collections::BTreeMap<String, u32>,
) -> Result<(), BackendError> {
    if value.get("type").and_then(Value::as_str) == Some("text") {
        let text = value
            .get("text")
            .and_then(Value::as_str)
            .ok_or_else(|| BackendError::InvalidEditorRevision("text node has no text".into()))?;
        let marks = value
            .get("marks")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let sensitive = marks
            .iter()
            .find(|mark| mark.get("type").and_then(Value::as_str) == Some("sensitive"));
        if let Some(mark) = sensitive {
            let attrs = mark.get("attrs").ok_or_else(|| {
                BackendError::InvalidEditorRevision("sensitive mark has no attrs".into())
            })?;
            let token_id = attrs
                .get("tokenId")
                .or_else(|| attrs.get("token_id"))
                .and_then(Value::as_str)
                .ok_or_else(|| {
                    BackendError::InvalidEditorRevision("sensitive mark has no token id".into())
                })?;
            validate_token_id(token_id)?;
            let source = attrs
                .get("source")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let mode = attrs
                .get("mode")
                .and_then(Value::as_str)
                .unwrap_or_default();
            if source == "manual" || mode == "manual-mask" || text != token_id {
                return Err(BackendError::InvalidEditorRevision(
                    "sensitive text is not a pipeline token".into(),
                ));
            }
            *marked.entry(token_id.to_owned()).or_default() += 1;
        } else if contains_hacienda_token(text) {
            return Err(BackendError::InvalidEditorRevision(
                "token-shaped text must carry a sensitive mark".into(),
            ));
        }
    }
    if let Some(children) = value.get("content") {
        let children = children.as_array().ok_or_else(|| {
            BackendError::InvalidEditorRevision("editor content must be an array".into())
        })?;
        for child in children {
            validate_editor_node(child, marked)?;
        }
    }
    Ok(())
}

fn contains_hacienda_token(text: &str) -> bool {
    !extract_hacienda_tokens(text).is_empty()
}

fn extract_hacienda_tokens(text: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut cursor = 0;
    while let Some(relative_start) = text[cursor..].find('[') {
        let start = cursor + relative_start;
        let Some(relative_end) = text[start..].find(']') else {
            break;
        };
        let end = start + relative_end + 1;
        let candidate = &text[start..end];
        if validate_token_id(candidate).is_ok() {
            tokens.push(candidate.to_owned());
        }
        cursor = end;
    }
    tokens
}

fn validate_token_id(token: &str) -> Result<(), BackendError> {
    let inner = token.strip_prefix('[').and_then(|v| v.strip_suffix(']'));
    let Some(inner) = inner else {
        return Err(BackendError::InvalidEditorRevision(
            "token is not bracketed".into(),
        ));
    };
    let fields = inner.split(':').collect::<Vec<_>>();
    if fields.len() != 3
        || fields[0].is_empty()
        || !fields[0]
            .bytes()
            .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit() || byte == b'_')
        || fields[1].is_empty()
        || !fields[1]
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
        || fields[2].is_empty()
        || !fields[2]
            .bytes()
            .all(|byte| byte.is_ascii_uppercase() || (b'2'..=b'7').contains(&byte))
    {
        return Err(BackendError::InvalidEditorRevision(
            "token is not a Hacienda pseudonym".into(),
        ));
    }
    Ok(())
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
    /// Open the private, durable LanceDB path declared by a valid LocalFull profile.
    /// TeamOnPremise deliberately has no equivalent constructor: its client receives
    /// a tenant-scoped [`RagStore`] from [`AuthenticatedTenantGateway`] instead.
    pub async fn connect_configured_lancedb(
        profile: InstallationConfig,
    ) -> Result<(Self, hacienda_rag::backends::lancedb::LanceDbStore), BackendError> {
        let backend = Self::new(profile)?;
        let path = match &backend.profile {
            InstallationConfig::LocalFull(config) => {
                let hacienda_client_core::LanceDbTopology::LocalEmbedded { path } =
                    &config.storage.lancedb
                else {
                    return Err(BackendError::InvalidLocalLanceDbTopology);
                };
                path.0.clone()
            }
            InstallationConfig::TeamOnPremise(_) => {
                return Err(BackendError::DirectLanceDbAccess);
            }
        };
        let store = hacienda_rag::backends::lancedb::LanceDbStore::connect(&path).await?;
        Ok((backend, store))
    }

    /// Explicit form that still enforces the profile's configured path; arbitrary
    /// client-supplied LanceDB URIs are rejected.
    pub async fn connect_lancedb(
        profile: InstallationConfig,
        uri: &str,
    ) -> Result<(Self, hacienda_rag::backends::lancedb::LanceDbStore), BackendError> {
        let configured = match &profile {
            InstallationConfig::LocalFull(config) => {
                let hacienda_client_core::LanceDbTopology::LocalEmbedded { path } =
                    &config.storage.lancedb
                else {
                    return Err(BackendError::InvalidLocalLanceDbTopology);
                };
                path.0.as_str()
            }
            InstallationConfig::TeamOnPremise(_) => return Err(BackendError::DirectLanceDbAccess),
        };
        if uri != configured {
            return Err(BackendError::UnconfiguredLanceDbPath);
        }
        Self::connect_configured_lancedb(profile).await
    }
}
