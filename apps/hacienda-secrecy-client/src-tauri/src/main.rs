#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
// `--all-targets` also builds this binary as a test harness, where `main()` and the
// Tauri command registration it references are compiled out (`#[cfg(not(test))]`).
// Those items are live in the shipped app; only the test-harness build sees them as dead.
#![cfg_attr(test, allow(dead_code))]

use futures::executor::block_on;
use hacienda_backend::{
    Backend, BackendDocument, RedactedXbergDocument, TrustedEditorRevisionIssuer,
};
use hacienda_core::{
    audit::{AuditEntryInput, AuditStore, EntitySource, RedactionAction},
    editor_projection::EditorEnvelope,
    pii::{PiiCategory, PiiPipeline, PipelineConfig},
    redaction::{KeyId, KeyResolver, PseudonymKey, Pseudonymiser, RedactionMode, KEY_BYTES},
    tenancy::TenantId,
    HaciendaConfig,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet},
    env,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
#[cfg(not(test))]
use tauri::Manager;
use tauri::{WebviewUrl, WebviewWindow, WebviewWindowBuilder};
use tokio::io::AsyncReadExt;
use zeroize::Zeroizing;

use hacienda_client_core::{
    ClaudeDesktopRequest, ClaudeDesktopResponse, ClientError, CodexRequest, CodexResponse,
    GatewayConfig, GatewayProviderTransport, InstallationConfig, OpaqueTokenReference,
    ProviderKind, ProviderRequest, ProviderResponse, ProviderStreamChunk, ProviderTransport,
    RedactedContext, RedactedProviderRequest, RedactionAdapter, RedactionError, SecretReference,
    TokenizedText, TransportError,
};
use hacienda_client_core::{
    ClientSurface, EncryptedLocalVault, GatewayId, LanceDbTopology, LocalFullConfig,
    LocalStorageTopology, ProviderRoute, ServiceEndpoint, StoragePath, TrustBoundary, VaultId,
};
#[cfg(not(test))]
use hacienda_core::audit::{FileAuditStore, NodeId};
use hacienda_rag::{
    LanceDbStore, RagStore, RetrieveMode, RetrieveQuery, TokenOnlyText, TokenTextEmbedder,
    XbergTokenEmbedder,
};

/// Opaque references are resolved inside the desktop process.  The renderer is
/// intentionally unable to send source bytes or extracted content here.
#[derive(Debug, Deserialize)]
struct OpenOriginalRequest {
    document_ref: String,
    version_ref: String,
    original_ref: String,
    viewer_ref: String,
    capability_ref: String,
}

#[derive(Debug, Serialize)]
struct OpenOriginalResponse {
    opened: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct OriginalBinding {
    document_ref: String,
    version_ref: String,
    original_ref: String,
    viewer_ref: String,
    capability_ref: String,
}

#[derive(Debug, Deserialize)]
struct LoadDocumentRequest {
    document_ref: String,
    version_ref: String,
}

#[derive(Debug, Deserialize)]
struct SaveDocumentRequest {
    document_ref: String,
    version_ref: String,
    editor: serde_json::Value,
}

#[derive(Clone, Debug, Serialize)]
struct DocumentSourceResponse {
    mime_type: String,
    original_ref: String,
    viewer_ref: String,
    capability_ref: String,
}

#[derive(Clone, Debug, Serialize)]
struct DocumentProjectionResponse {
    schema_version: String,
    document_id: String,
    version_id: String,
    source: DocumentSourceResponse,
    metadata_token_only: serde_json::Value,
    editor: serde_json::Value,
    search_projection: serde_json::Value,
    provenance: serde_json::Value,
}

#[derive(Clone, Debug, Serialize)]
struct SaveDocumentResponse {
    document_id: String,
    version_id: String,
    parent_version_id: String,
    idempotent: bool,
}

#[derive(Debug, Deserialize)]
struct LocalSearchRequest {
    query: String,
}

#[derive(Debug, Serialize)]
struct LocalSearchResult {
    title_token_only: String,
    content_token_only: String,
    ordinal: u32,
    score: f32,
}

#[derive(Debug, Serialize)]
struct LocalSearchResponse {
    results: Vec<LocalSearchResult>,
}

const LOCAL_KEYRING_SERVICE: &str = "org.hacienda.secrecy-client";
const LOCAL_KEYRING_USER: &str = "local-full-pseudonym-key-v1";
const NER_MODEL_DIR_ENV: &str = "HACIENDA_NER_MODEL_DIR";
const NER_MODEL_APP_DATA_RELATIVE_DIR: &str = "models/gliner2-pii";
const PINNED_NER_MODEL_ID: &str = "fastino/gliner2-privacy-filter-PII-multi";
const LOCAL_EMBEDDING_PRESET: &str = "multilingual";
const LOCAL_EMBEDDING_VERSION: u32 = 1;
#[cfg(all(target_os = "macos", target_arch = "x86_64"))]
const ONNXRUNTIME_RESOURCE_TARGET: &str = "macos-x86_64";
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
const ONNXRUNTIME_RESOURCE_TARGET: &str = "macos-aarch64";
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
const ONNXRUNTIME_RESOURCE_TARGET: &str = "linux-x86_64";
#[cfg(all(target_os = "linux", target_arch = "aarch64"))]
const ONNXRUNTIME_RESOURCE_TARGET: &str = "linux-aarch64";
#[cfg(all(target_os = "windows", target_arch = "x86_64"))]
const ONNXRUNTIME_RESOURCE_TARGET: &str = "windows-x86_64";
#[cfg(all(target_os = "windows", target_arch = "aarch64"))]
const ONNXRUNTIME_RESOURCE_TARGET: &str = "windows-aarch64";

#[cfg(target_os = "macos")]
const ONNXRUNTIME_LIBRARY_FILE: &str = "libonnxruntime.1.23.2.dylib";
#[cfg(target_os = "linux")]
const ONNXRUNTIME_LIBRARY_FILE: &str = "libonnxruntime.so.1.23.2";
#[cfg(target_os = "windows")]
const ONNXRUNTIME_LIBRARY_FILE: &str = "onnxruntime.dll";
const LOCAL_EMBEDDING_COLLECTION: &str = "documents-multilingual-v1";

// The asset allowlist now lives in `model-assets/registry.json`, shared with
// Studio's `model-digests.ts` and `hacienda-core`'s native loader through
// `hacienda-model-assets`. Runtime validation deliberately does not trust a
// model-provided manifest; it checks the pinned bytes and then the architecture.

#[derive(Debug)]
enum NerModelError {
    Unavailable,
    Invalid,
}

fn resolve_ner_model_dir(app_data_dir: Option<&Path>) -> Result<PathBuf, NerModelError> {
    if let Some(path) = env::var_os(NER_MODEL_DIR_ENV) {
        let path = PathBuf::from(path);
        return (!path.as_os_str().is_empty())
            .then_some(path)
            .ok_or(NerModelError::Unavailable);
    }
    app_data_dir
        .map(|root| root.join(NER_MODEL_APP_DATA_RELATIVE_DIR))
        .ok_or(NerModelError::Unavailable)
}

fn validate_ner_model_assets(model_dir: &Path) -> Result<(), NerModelError> {
    if !model_dir.is_dir() {
        return Err(NerModelError::Unavailable);
    }
    validate_optional_manifest_identity(model_dir)?;
    hacienda_model_assets::verify_dir(PINNED_NER_MODEL_ID, model_dir).map_err(|error| {
        eprintln!("[ner] pinned model asset verification failed: {error}");
        NerModelError::Invalid
    })?;
    validate_ner_model_architecture(model_dir)
}

fn validate_optional_manifest_identity(model_dir: &Path) -> Result<(), NerModelError> {
    let expected_revision = hacienda_model_assets::model_entry(PINNED_NER_MODEL_ID)
        .and_then(|entry| entry.revision.as_deref())
        .ok_or(NerModelError::Invalid)?;
    let path = model_dir.join("manifest.json");
    if !path.is_file() {
        return Ok(());
    }
    let manifest = serde_json::from_slice::<serde_json::Value>(
        &std::fs::read(path).map_err(|_| NerModelError::Invalid)?,
    )
    .map_err(|_| NerModelError::Invalid)?;
    if manifest.get("model_id").and_then(serde_json::Value::as_str) != Some(PINNED_NER_MODEL_ID)
        || manifest.get("revision").and_then(serde_json::Value::as_str)
            != Some(expected_revision)
    {
        return Err(NerModelError::Invalid);
    }
    Ok(())
}

fn validate_ner_model_architecture(model_dir: &Path) -> Result<(), NerModelError> {
    let config = read_json_asset(model_dir, "config.json")?;
    if config.get("model_type").and_then(serde_json::Value::as_str) != Some("extractor")
        || config
            .get("counting_layer")
            .and_then(serde_json::Value::as_str)
            != Some("count_lstm")
        || config.get("max_width").and_then(serde_json::Value::as_u64) != Some(8)
        || config
            .get("token_pooling")
            .and_then(serde_json::Value::as_str)
            != Some("first")
        || config.get("model_name").and_then(serde_json::Value::as_str)
            != Some("microsoft/mdeberta-v3-base")
    {
        return Err(NerModelError::Invalid);
    }
    let encoder = read_json_asset(model_dir, "encoder_config/config.json")?;
    if encoder
        .get("model_type")
        .and_then(serde_json::Value::as_str)
        != Some("deberta-v2")
        || encoder
            .get("hidden_size")
            .and_then(serde_json::Value::as_u64)
            != Some(768)
        || encoder
            .get("num_hidden_layers")
            .and_then(serde_json::Value::as_u64)
            != Some(12)
        || encoder
            .get("vocab_size")
            .and_then(serde_json::Value::as_u64)
            != Some(250_112)
    {
        return Err(NerModelError::Invalid);
    }
    let tokenizer = read_json_asset(model_dir, "tokenizer_config.json")?;
    if tokenizer
        .get("tokenizer_class")
        .and_then(serde_json::Value::as_str)
        != Some("DebertaV2Tokenizer")
    {
        return Err(NerModelError::Invalid);
    }
    Ok(())
}

fn read_json_asset(
    model_dir: &Path,
    relative_path: &str,
) -> Result<serde_json::Value, NerModelError> {
    let bytes = std::fs::read(model_dir.join(relative_path)).map_err(|_| NerModelError::Invalid)?;
    serde_json::from_slice(&bytes).map_err(|_| NerModelError::Invalid)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum VaultError {
    Locked,
    InvalidKey,
    RandomSourceUnavailable,
}

trait LocalPseudonymKeyStore: Send + Sync {
    fn load_or_create(&self) -> Result<Zeroizing<[u8; KEY_BYTES]>, VaultError>;
}

/// The OS credential store is the only production source of the LocalFull key.
/// No file, environment variable, renderer state, or provider configuration is used.
struct OsCredentialKeyStore;

impl LocalPseudonymKeyStore for OsCredentialKeyStore {
    fn load_or_create(&self) -> Result<Zeroizing<[u8; KEY_BYTES]>, VaultError> {
        let entry = keyring::Entry::new(LOCAL_KEYRING_SERVICE, LOCAL_KEYRING_USER)
            .map_err(|_| VaultError::Locked)?;
        match entry.get_secret() {
            Ok(secret) => key_from_bytes(secret),
            Err(keyring::Error::NoEntry) => {
                let mut secret = Zeroizing::new([0_u8; KEY_BYTES]);
                getrandom::fill(&mut *secret).map_err(|_| VaultError::RandomSourceUnavailable)?;
                entry
                    .set_secret(secret.as_ref())
                    .map_err(|_| VaultError::Locked)?;
                // Re-read after creation so concurrent first launches converge on the
                // item actually committed to the OS store.
                let stored = entry.get_secret().map_err(|_| VaultError::Locked)?;
                key_from_bytes(stored)
            }
            Err(_) => Err(VaultError::Locked),
        }
    }
}

fn key_from_bytes(secret: Vec<u8>) -> Result<Zeroizing<[u8; KEY_BYTES]>, VaultError> {
    let secret = Zeroizing::new(secret);
    let key: [u8; KEY_BYTES] = secret
        .as_slice()
        .try_into()
        .map_err(|_| VaultError::InvalidKey)?;
    Ok(Zeroizing::new(key))
}

struct LocalKeyResolver(Zeroizing<[u8; KEY_BYTES]>);

impl KeyResolver for LocalKeyResolver {
    fn active(
        &self,
        _tenant: &TenantId,
    ) -> Result<PseudonymKey, hacienda_core::redaction::PseudonymError> {
        PseudonymKey::from_bytes(KeyId::new("local")?, self.0.as_ref())
    }

    fn resolve(
        &self,
        _tenant: &TenantId,
        id: &KeyId,
    ) -> Result<PseudonymKey, hacienda_core::redaction::PseudonymError> {
        if id.as_str() != "local" {
            return Err(hacienda_core::redaction::PseudonymError::KeyNotFound {
                id: id.as_str().to_owned(),
                variable: "LocalFull OS credential store".to_owned(),
            });
        }
        self.active(_tenant)
    }
}

fn local_full_profile(lancedb_path: &Path) -> InstallationConfig {
    InstallationConfig::LocalFull(LocalFullConfig {
        client: ClientSurface::LocalTauri,
        trust_boundary: TrustBoundary::LocalProcess,
        hacienda: Some(ServiceEndpoint::new("http://127.0.0.1:0")),
        xberg: Some(ServiceEndpoint::new("http://127.0.0.1:0")),
        gateway: Some(GatewayConfig::Local {
            id: GatewayId("local-full".into()),
            endpoint: ServiceEndpoint::new("http://127.0.0.1:0"),
        }),
        storage: LocalStorageTopology {
            vault: Some(EncryptedLocalVault {
                id: VaultId("local-full-vault".into()),
                path: StoragePath("<native-vault>".into()),
                key_reference: SecretReference("os-keychain:local-full".into()),
            }),
            lancedb: LanceDbTopology::LocalEmbedded {
                path: StoragePath(lancedb_path.to_string_lossy().into_owned()),
            },
        },
        provider_route: ProviderRoute::RedactedViaGateway {
            gateway: GatewayId("local-full".into()),
        },
    })
}

#[cfg(all(feature = "ner-candle", not(target_arch = "wasm32")))]
fn configured_document_runtime(
    key: &Zeroizing<[u8; KEY_BYTES]>,
    app_data_dir: &Path,
) -> Result<(hacienda_core::HaciendaFacade, TrustedEditorRevisionIssuer), String> {
    let model_dir = resolve_ner_model_dir(Some(app_data_dir))
        .and_then(|path| validate_ner_model_assets(&path).map(|()| path))
        .map_err(|_| "LocalFull PII model is unavailable".to_owned())?;
    let mut pipeline_config = PipelineConfig::default();
    pipeline_config.redaction.mode = RedactionMode::Pseudonymize;
    pipeline_config.model.enabled = true;
    pipeline_config.model.model_dir = Some(model_dir);
    let resolver = Arc::new(LocalKeyResolver(Zeroizing::new(**key)));
    let facade = HaciendaConfig::default().with_pii(pipeline_config.clone());
    let facade = hacienda_core::HaciendaFacade::with_key_resolver(
        facade,
        Arc::clone(&resolver) as Arc<dyn KeyResolver>,
        &[],
    )
    .map_err(|_| "LocalFull Hacienda/Xberg runtime is unavailable".to_owned())?;
    let pseudonymiser = Arc::new(
        Pseudonymiser::new(resolver.as_ref(), &TenantId::default_tenant(), &[])
            .map_err(|_| "LocalFull pseudonym vault is unavailable".to_owned())?,
    );
    let pipeline = Arc::new(
        PiiPipeline::with_pseudonymiser(pipeline_config, Some(pseudonymiser))
            .map_err(|_| "LocalFull PII pipeline is unavailable".to_owned())?,
    );
    let issuer = TrustedEditorRevisionIssuer::new(pipeline)
        .map_err(|_| "LocalFull editor pipeline is unavailable".to_owned())?;
    Ok((facade, issuer))
}

#[cfg(any(not(feature = "ner-candle"), target_arch = "wasm32"))]
fn configured_document_runtime(
    _key: &Zeroizing<[u8; KEY_BYTES]>,
    _app_data_dir: &Path,
) -> Result<(hacienda_core::HaciendaFacade, TrustedEditorRevisionIssuer), String> {
    Err("LocalFull document ingestion requires the real NER model feature".to_owned())
}

struct StoredLocalDocument {
    backend_document: BackendDocument,
    binding: OriginalBinding,
    original_path: PathBuf,
}

struct LocalFullDocumentService {
    backend: Backend,
    store: Arc<LanceDbStore>,
    embedding_config: xberg::EmbeddingConfig,
    embedder: XbergTokenEmbedder,
    embedding_ready: bool,
    audit: Arc<dyn AuditStore>,
    facade: hacienda_core::HaciendaFacade,
    issuer: TrustedEditorRevisionIssuer,
    documents: HashMap<String, StoredLocalDocument>,
}

impl LocalFullDocumentService {
    async fn new(root: &Path, audit: Arc<dyn AuditStore>) -> Result<Self, String> {
        let runtime_root = root.to_path_buf();
        let (facade, issuer) = tokio::task::spawn_blocking(move || {
            let key = OsCredentialKeyStore
                .load_or_create()
                .map_err(|_| "LocalFull pseudonym vault is unavailable".to_owned())?;
            configured_document_runtime(&key, &runtime_root)
        })
        .await
        .map_err(|_| "LocalFull document runtime initialization failed".to_owned())??;
        let profile = local_full_profile(&root.join("lancedb"));
        let (backend, store) = Backend::connect_configured_lancedb(profile)
            .await
            .map_err(|_| "LocalFull LanceDB is unavailable".to_owned())?;
        let embedding_cache = root.join("models/xberg-embeddings");
        tokio::fs::create_dir_all(&embedding_cache)
            .await
            .map_err(|_| "LocalFull embedding cache is unavailable".to_owned())?;
        #[cfg(all(target_os = "macos", target_arch = "x86_64"))]
        let acceleration = Some(xberg::AccelerationConfig {
            provider: xberg::ExecutionProviderType::Cpu,
            ..Default::default()
        });
        #[cfg(not(all(target_os = "macos", target_arch = "x86_64")))]
        let acceleration = None;
        let embedding_config = xberg::EmbeddingConfig {
            model: xberg::EmbeddingModelType::Preset {
                name: LOCAL_EMBEDDING_PRESET.to_owned(),
            },
            cache_dir: Some(embedding_cache),
            acceleration,
            ..Default::default()
        };
        let embedder =
            XbergTokenEmbedder::from_config(embedding_config.clone(), LOCAL_EMBEDDING_VERSION)
                .map_err(|_| "LocalFull embedding model configuration is unavailable".to_owned())?;
        Ok(Self {
            backend,
            store: Arc::new(store),
            embedding_config,
            embedder,
            embedding_ready: false,
            audit,
            facade,
            issuer,
            documents: HashMap::new(),
        })
    }

    async fn ingest_path(
        &mut self,
        path: PathBuf,
    ) -> Result<(DocumentProjectionResponse, OriginalBinding, PathBuf), String> {
        if !self.embedding_ready {
            return Err(
                "Install or activate the local multilingual retrieval model first".to_owned(),
            );
        }
        let filename = path
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| "Selected document filename is invalid".to_owned())?
            .to_owned();
        let mime_type = mime_type_for_filename(&filename);
        let bytes = read_selected_document(&path).await?;
        let result = self
            .facade
            .process(xberg::ExtractInput::from_bytes(
                bytes,
                mime_type,
                Some(filename),
            ))
            .await
            .map_err(|_| "LocalFull extraction or redaction failed closed".to_owned())?;
        let redacted = RedactedXbergDocument::from_hacienda_result(&result, 0)
            .map_err(|_| "LocalFull extraction did not produce a redacted document".to_owned())?;
        let document_ref = opaque_reference("doc");
        let version_ref = opaque_reference("ver");
        let document = self
            .backend
            .project(
                &redacted,
                "local",
                document_ref.clone(),
                version_ref.clone(),
            )
            .map_err(|_| "Token-only document projection failed".to_owned())?;
        self.backend
            .persist_to_with_embedder(
                self.store.as_ref(),
                LOCAL_EMBEDDING_COLLECTION,
                &document,
                &self.embedder,
            )
            .await
            .map_err(|_| "Token-only document embedding or persistence failed".to_owned())?;
        let binding = OriginalBinding {
            document_ref: document_ref.clone(),
            version_ref: version_ref.clone(),
            original_ref: opaque_reference("orig"),
            viewer_ref: opaque_reference("viewer"),
            capability_ref: opaque_reference("cap"),
        };
        let projection = document_projection(document.editor(), &binding)?;
        self.documents.insert(
            document_ref,
            StoredLocalDocument {
                backend_document: document,
                binding: binding.clone(),
                original_path: path.clone(),
            },
        );
        Ok((projection, binding, path))
    }

    fn load(&self, request: &LoadDocumentRequest) -> Result<DocumentProjectionResponse, String> {
        let stored = self
            .documents
            .get(&request.document_ref)
            .ok_or_else(|| "Document is not loaded in this LocalFull session".to_owned())?;
        if stored.backend_document.editor().version_ref != request.version_ref {
            return Err("Document version is not available".to_owned());
        }
        document_projection(stored.backend_document.editor(), &stored.binding)
    }

    fn binding_for(&self, document_ref: &str) -> Result<(OriginalBinding, PathBuf), String> {
        let stored = self
            .documents
            .get(document_ref)
            .ok_or_else(|| "Document is not loaded in this LocalFull session".to_owned())?;
        Ok((stored.binding.clone(), stored.original_path.clone()))
    }

    async fn save(&mut self, request: SaveDocumentRequest) -> Result<SaveDocumentResponse, String> {
        let base = self
            .documents
            .get(&request.document_ref)
            .ok_or_else(|| "Document is not loaded in this LocalFull session".to_owned())?;
        if base.backend_document.editor().version_ref != request.version_ref {
            return Err("Document version is stale".to_owned());
        }
        let base_document = base.backend_document.clone();
        let revision = self
            .issuer
            .issue(&base_document, &request.editor)
            .await
            .map_err(|_| "Editor save failed closed at the PII pipeline".to_owned())?;
        let saved = self
            .backend
            .append_editor_revision_with_embedder(
                self.audit.as_ref(),
                self.store.as_ref(),
                LOCAL_EMBEDDING_COLLECTION,
                &base_document,
                &revision,
                &self.embedder,
            )
            .await
            .map_err(|_| "Editor version could not be appended".to_owned())?;
        let next_document = self
            .backend
            .document_for_editor_revision(&base_document, &revision, &saved.version_id)
            .map_err(|_| "Saved version projection failed".to_owned())?;
        if let Some(stored) = self.documents.get_mut(&request.document_ref) {
            stored.backend_document = next_document;
            stored.binding.version_ref = saved.version_id.clone();
            stored.binding.capability_ref = opaque_reference("cap");
        }
        Ok(SaveDocumentResponse {
            document_id: saved.document_id,
            version_id: saved.version_id,
            parent_version_id: request.version_ref,
            idempotent: saved.idempotent,
        })
    }

    async fn install_embedding_model(&mut self) -> Result<(), String> {
        let model = self.embedding_config.model.clone();
        let cache_dir = self.embedding_config.cache_dir.clone();
        tokio::task::spawn_blocking(move || xberg::embeddings::warm_model(&model, cache_dir))
            .await
            .map_err(|_| "LocalFull embedding model initialization failed".to_owned())?
            .map_err(|_| "LocalFull multilingual embedding model could not be loaded".to_owned())?;
        self.embedding_ready = true;
        Ok(())
    }

    async fn redact_local_text(&self, text: &str) -> Result<String, String> {
        if text.trim().is_empty() || text.chars().count() > 16_000 {
            return Err("Local search text is empty or exceeds the supported length".to_owned());
        }
        let result = self
            .facade
            .process(xberg::ExtractInput::from_bytes(
                text.as_bytes().to_vec(),
                "text/plain",
                None,
            ))
            .await
            .map_err(|_| "Local search redaction failed closed".to_owned())?;
        if !result.metadata.pii_enabled || result.pii.len() != 1 {
            return Err("Complete local redaction is unavailable for search".to_owned());
        }
        result
            .pii
            .first()
            .map(|pipeline| pipeline.redacted_text.clone())
            .filter(|redacted| !redacted.trim().is_empty())
            .ok_or_else(|| "Local search redaction returned no safe text".to_owned())
    }

    async fn search(&self, request: LocalSearchRequest) -> Result<LocalSearchResponse, String> {
        if !self.embedding_ready {
            return Err("Activate the local multilingual retrieval model first".to_owned());
        }
        let tokenized_query = self.redact_local_text(&request.query).await?;
        let query_vector = self
            .embedder
            .embed(&[TokenOnlyText::from_token_only(tokenized_query.clone())])
            .await
            .map_err(|_| "Local search embedding failed closed".to_owned())?
            .into_iter()
            .next()
            .ok_or_else(|| "Local search model returned no vector".to_owned())?;
        let collection = self
            .backend
            .collection_name(LOCAL_EMBEDDING_COLLECTION)
            .map_err(|_| "Local search collection is unavailable".to_owned())?;
        let output = self
            .store
            .retrieve(
                &collection,
                &RetrieveQuery {
                    mode: RetrieveMode::Hybrid,
                    query_text: Some(tokenized_query),
                    query_vector: Some(query_vector),
                    top_k: 8,
                    filter: None,
                    candidate_multiplier: Some(4),
                    group_by_document: true,
                    include_content: true,
                    include_document: true,
                    ..RetrieveQuery::vector(8)
                },
            )
            .await
            .map_err(|_| "Local hybrid search failed closed".to_owned())?;
        let mut results = Vec::with_capacity(output.chunks.len());
        for chunk in output.chunks {
            let Some(content) = chunk.content else {
                continue;
            };
            // Re-redact data read from LanceDB before it crosses IPC. This guards the
            // renderer and provider path against stale, malformed, or externally altered rows.
            let content_token_only = self.redact_local_text(&content).await?;
            let title = chunk
                .document
                .as_ref()
                .and_then(|document| document.title.as_deref())
                .filter(|title| !title.trim().is_empty())
                .unwrap_or("Document sans titre");
            let title_token_only = self.redact_local_text(title).await?;
            if !chunk.score.is_finite() {
                return Err("Local search returned an invalid score".to_owned());
            }
            results.push(LocalSearchResult {
                title_token_only,
                content_token_only,
                ordinal: chunk.ordinal,
                score: chunk.score,
            });
        }
        Ok(LocalSearchResponse { results })
    }
}

async fn read_selected_document(path: &Path) -> Result<Vec<u8>, String> {
    let limit = xberg::SecurityLimits::default().max_archive_size;
    let file = tokio::fs::File::open(path)
        .await
        .map_err(|_| "Selected document could not be read".to_owned())?;
    let metadata = file
        .metadata()
        .await
        .map_err(|_| "Selected document could not be read".to_owned())?;
    if metadata.len() > limit as u64 {
        return Err("Selected document exceeds the size limit".to_owned());
    }
    let mut bytes = Vec::new();
    file.take(limit as u64 + 1)
        .read_to_end(&mut bytes)
        .await
        .map_err(|_| "Selected document could not be read".to_owned())?;
    if bytes.len() > limit {
        return Err("Selected document exceeds the size limit".to_owned());
    }
    Ok(bytes)
}

fn opaque_reference(prefix: &str) -> String {
    let digest = blake3::hash(format!("{prefix}:{}", uuid::Uuid::new_v4()).as_bytes()).to_hex();
    format!("{prefix}_{}", &digest[..24])
}

fn mime_type_for_filename(filename: &str) -> String {
    match filename
        .rsplit('.')
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase()
        .as_str()
    {
        "pdf" => "application/pdf",
        "docx" => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        "xlsx" => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        "pptx" => "application/vnd.openxmlformats-officedocument.presentationml.presentation",
        "txt" | "md" | "markdown" => "text/plain",
        "csv" => "text/csv",
        "json" => "application/json",
        _ => "application/octet-stream",
    }
    .to_owned()
}

fn document_projection(
    editor: &EditorEnvelope,
    binding: &OriginalBinding,
) -> Result<DocumentProjectionResponse, String> {
    Ok(DocumentProjectionResponse {
        schema_version: editor.schema.clone(),
        document_id: editor.document_ref.clone(),
        version_id: editor.version_ref.clone(),
        source: DocumentSourceResponse {
            mime_type: editor.metadata.legal.mime_type.clone(),
            original_ref: binding.original_ref.clone(),
            viewer_ref: binding.viewer_ref.clone(),
            capability_ref: binding.capability_ref.clone(),
        },
        metadata_token_only: serde_json::to_value(&editor.metadata)
            .map_err(|_| "Metadata projection is invalid".to_owned())?,
        editor: editor.editor.clone(),
        search_projection: serde_json::to_value(&editor.search)
            .map_err(|_| "Search projection is invalid".to_owned())?,
        provenance: serde_json::to_value(&editor.provenance)
            .map_err(|_| "Provenance projection is invalid".to_owned())?,
    })
}

struct LocalFullDocumentState {
    root: PathBuf,
    audit: Option<Arc<dyn AuditStore>>,
    service: tokio::sync::Mutex<Option<LocalFullDocumentService>>,
}

impl LocalFullDocumentState {
    fn new(root: PathBuf, audit: Option<Arc<dyn AuditStore>>) -> Self {
        Self {
            root,
            audit,
            service: tokio::sync::Mutex::const_new(None),
        }
    }

    async fn service(
        &self,
    ) -> Result<tokio::sync::MutexGuard<'_, Option<LocalFullDocumentService>>, String> {
        let mut service = self.service.lock().await;
        if service.is_none() {
            let audit = self
                .audit
                .as_ref()
                .map(Arc::clone)
                .ok_or_else(|| "LocalFull audit store is unavailable".to_owned())?;
            *service = Some(LocalFullDocumentService::new(&self.root, audit).await?);
        }
        Ok(service)
    }
}

const MAIN_WINDOW_LABEL: &str = "main";
fn require_main_window(window: &WebviewWindow) -> Result<(), String> {
    if window.label() == MAIN_WINDOW_LABEL {
        Ok(())
    } else {
        Err("Main window access is required".to_owned())
    }
}
const LOCAL_VIEWER_PREFIX: &str = "local-viewer-";
const VIEWER_SESSION_TTL: Duration = Duration::from_secs(60);
/// Rust-only one-shot authority minted after the caller window, session, and token
/// membership have been checked. It is never serialized or accepted from a renderer.
pub struct ViewerCapability {
    viewer_label: String,
    token: String,
    expires_at: Instant,
    consumed: bool,
}

impl ViewerCapability {
    fn issue(viewer_label: &str, token: &str, expires_at: Instant) -> Self {
        Self {
            viewer_label: viewer_label.to_owned(),
            token: token.to_owned(),
            expires_at,
            consumed: false,
        }
    }

    fn consume(&mut self, viewer_label: &str, token: &str) -> Result<(), RedactionError> {
        if self.consumed
            || Instant::now() >= self.expires_at
            || self.viewer_label != viewer_label
            || self.token != token
        {
            return Err(RedactionError::Rejected);
        }
        self.consumed = true;
        Ok(())
    }
}

/// LocalFull uses Hacienda's authenticated pseudonym tokens. The OS credential store
/// supplies the stable key; failure to open it disables this adapter.
struct HaciendaLocalRedactor {
    pipeline: PiiPipeline,
    pseudonymiser: Arc<Pseudonymiser>,
    pii_detection_complete: bool,
}

impl HaciendaLocalRedactor {
    fn new() -> Result<Self, RedactionError> {
        Self::from_key_store_at(Arc::new(OsCredentialKeyStore), None)
    }

    fn new_with_app_data_dir(app_data_dir: &Path) -> Result<Self, RedactionError> {
        Self::from_key_store_at(Arc::new(OsCredentialKeyStore), Some(app_data_dir))
    }

    /// Test-only compatibility seam for viewer/token tests that do not exercise
    /// provider egress. Production construction goes through `new` and requires
    /// a validated, loaded local model before it can authorize egress.
    #[cfg(test)]
    fn from_key_store(store: Arc<dyn LocalPseudonymKeyStore>) -> Result<Self, RedactionError> {
        Self::from_key_store_without_model(store)
    }

    #[cfg(test)]
    fn from_key_store_without_model(
        store: Arc<dyn LocalPseudonymKeyStore>,
    ) -> Result<Self, RedactionError> {
        let key = store
            .load_or_create()
            .map_err(|_| RedactionError::Rejected)?;
        let resolver = LocalKeyResolver(key);
        let pseudonymiser = Arc::new(
            Pseudonymiser::new(&resolver, &TenantId::default_tenant(), &[])
                .map_err(|_| RedactionError::Rejected)?,
        );
        let mut config = PipelineConfig::default();
        config.redaction.mode = RedactionMode::Pseudonymize;
        // This Tauri package does not build the `ner-candle` backend or provision
        // verified GLiNER weights, so regex-only detection can never authorize egress.
        let pii_detection_complete = false;
        let pipeline = PiiPipeline::with_pseudonymiser(config, Some(pseudonymiser.clone()))
            .map_err(|_| RedactionError::Rejected)?;
        Ok(Self {
            pipeline,
            pseudonymiser,
            pii_detection_complete,
        })
    }

    fn from_key_store_at(
        store: Arc<dyn LocalPseudonymKeyStore>,
        app_data_dir: Option<&Path>,
    ) -> Result<Self, RedactionError> {
        let model_dir =
            resolve_ner_model_dir(app_data_dir).map_err(|_| RedactionError::Rejected)?;
        Self::from_key_store_with_runtime(
            store,
            &model_dir,
            validate_ner_model_assets,
            |config, pseudonymiser| PiiPipeline::with_pseudonymiser(config, Some(pseudonymiser)),
        )
    }

    fn from_key_store_with_runtime<Validate, Build>(
        store: Arc<dyn LocalPseudonymKeyStore>,
        model_dir: &Path,
        validate: Validate,
        build: Build,
    ) -> Result<Self, RedactionError>
    where
        Validate: FnOnce(&Path) -> Result<(), NerModelError>,
        Build: FnOnce(
            PipelineConfig,
            Arc<Pseudonymiser>,
        ) -> Result<PiiPipeline, hacienda_core::pii::PiiError>,
    {
        validate(model_dir).map_err(|_| RedactionError::Rejected)?;
        let key = store
            .load_or_create()
            .map_err(|_| RedactionError::Rejected)?;
        let resolver = Arc::new(LocalKeyResolver(key));
        let pseudonymiser = Arc::new(
            Pseudonymiser::new(resolver.as_ref(), &TenantId::default_tenant(), &[])
                .map_err(|_| RedactionError::Rejected)?,
        );
        let mut config = PipelineConfig::default();
        config.redaction.mode = RedactionMode::Pseudonymize;
        config.model.enabled = true;
        config.model.model_dir = Some(model_dir.to_owned());
        let pipeline =
            build(config, Arc::clone(&pseudonymiser)).map_err(|_| RedactionError::Rejected)?;
        let ready = pipeline.config().model.enabled
            && pipeline.config().model.model_dir.is_some()
            && pipeline.config().redaction.mode == RedactionMode::Pseudonymize;
        if !ready {
            return Err(RedactionError::Rejected);
        }
        Ok(Self {
            pipeline,
            pseudonymiser,
            pii_detection_complete: true,
        })
    }

    fn tokenize(&self, text: &str) -> Result<TokenizedText, RedactionError> {
        let result = block_on(self.pipeline.process(text)).map_err(|_| RedactionError::Rejected)?;
        let references = emitted_token_references(&result.redacted_text, &self.pseudonymiser);
        Ok(TokenizedText::new(result.redacted_text, references))
    }

    fn reveal(
        &self,
        token: &str,
        viewer_label: &str,
        capability: &mut ViewerCapability,
    ) -> Result<Zeroizing<String>, RedactionError> {
        capability.consume(viewer_label, token)?;
        let revealed = self
            .pseudonymiser
            .reveal(token)
            .map(Zeroizing::new)
            .map_err(|_| RedactionError::Rejected)?;
        Ok(revealed)
    }
}

fn emitted_token_references(
    text: &str,
    pseudonymiser: &Pseudonymiser,
) -> Vec<OpaqueTokenReference> {
    let mut references = Vec::new();
    let mut remaining = text;
    while let Some(start) = remaining.find('[') {
        let candidate = &remaining[start..];
        let Some(end) = candidate.find(']') else {
            break;
        };
        let token = &candidate[..=end];
        let category = token
            .strip_prefix('[')
            .and_then(|body| body.split_once(':'))
            .map(|(category, _)| category);
        let authenticated = pseudonymiser.reveal(token).map(Zeroizing::new);
        if category.is_some() && authenticated.is_ok() {
            let reference = OpaqueTokenReference::new(token, category.unwrap_or_default());
            if !references.contains(&reference) {
                references.push(reference);
            }
        }
        remaining = &candidate[end + 1..];
    }
    references
}

#[derive(Debug, Deserialize)]
struct OpenLocalViewerRequest {
    tokenized: TokenizedText,
    references: Vec<OpaqueTokenReference>,
}

#[derive(Clone, Debug, Serialize)]
struct OpenLocalViewerResponse {
    opened: bool,
}

#[derive(Clone, Debug, Serialize)]
struct LocalViewerContentResponse {
    tokenized: TokenizedText,
    references: Vec<OpaqueTokenReference>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum ViewerCommandError {
    Unauthorized,
    InvalidTokenizedPayload,
    InvalidCategory,
    EmptyValue,
    Expired,
    Replayed,
    TokenNotInSession,
    Unavailable,
    WindowOpenFailed,
}

struct ViewerSession {
    tokenized: TokenizedText,
    references: HashSet<OpaqueTokenReference>,
    expires_at: Instant,
    consumed_tokens: HashSet<String>,
}

struct LocalViewerState {
    redactor: Option<Arc<HaciendaLocalRedactor>>,
    sessions: Mutex<HashMap<String, ViewerSession>>,
    audit_store: Option<Arc<dyn AuditStore>>,
    audit_tenant: TenantId,
}

#[derive(Clone, Debug, Serialize)]
struct LocalPiiStatusResponse {
    provider_send_enabled: bool,
    detector: &'static str,
}

impl LocalViewerState {
    fn new(redactor: Option<Arc<HaciendaLocalRedactor>>) -> Self {
        Self {
            redactor,
            sessions: Mutex::new(HashMap::new()),
            audit_store: None,
            audit_tenant: TenantId::new("local-full"),
        }
    }

    fn with_audit_store(mut self, audit_store: Arc<dyn AuditStore>) -> Self {
        self.audit_store = Some(audit_store);
        self
    }

    #[cfg(test)]
    fn fail_closed() -> Self {
        Self::new(None)
    }

    fn pii_status(&self, provider_send_configured: bool) -> LocalPiiStatusResponse {
        let complete = self
            .redactor
            .as_ref()
            .is_some_and(|redactor| redactor.pii_detection_complete);
        LocalPiiStatusResponse {
            provider_send_enabled: complete && provider_send_configured,
            detector: if complete {
                "local_model"
            } else {
                "regex_only"
            },
        }
    }

    fn open_session(
        &self,
        caller_label: &str,
        request: OpenLocalViewerRequest,
    ) -> Result<String, ViewerCommandError> {
        if caller_label != MAIN_WINDOW_LABEL {
            return Err(ViewerCommandError::Unauthorized);
        }
        let redactor = self
            .redactor
            .as_ref()
            .ok_or(ViewerCommandError::Unavailable)?;
        let validated = validate_viewer_payload(redactor, request)?;
        let label = new_viewer_label()?;
        let mut sessions = self
            .sessions
            .lock()
            .map_err(|_| ViewerCommandError::Unavailable)?;
        sessions.retain(|_, session| session.expires_at > Instant::now());
        sessions.insert(
            label.clone(),
            ViewerSession {
                tokenized: validated.tokenized,
                references: validated.references,
                expires_at: Instant::now() + VIEWER_SESSION_TTL,
                consumed_tokens: HashSet::new(),
            },
        );
        Ok(label)
    }

    #[cfg(test)]
    fn open_expired_session_for_test(
        &self,
        request: OpenLocalViewerRequest,
    ) -> Result<String, ViewerCommandError> {
        let redactor = self
            .redactor
            .as_ref()
            .ok_or(ViewerCommandError::Unavailable)?;
        let validated = validate_viewer_payload(redactor, request)?;
        let label = new_viewer_label()?;
        self.sessions
            .lock()
            .map_err(|_| ViewerCommandError::Unavailable)?
            .insert(
                label.clone(),
                ViewerSession {
                    tokenized: validated.tokenized,
                    references: validated.references,
                    expires_at: Instant::now() - Duration::from_secs(1),
                    consumed_tokens: HashSet::new(),
                },
            );
        Ok(label)
    }

    fn content_for_viewer(
        &self,
        viewer_label: &str,
    ) -> Result<LocalViewerContentResponse, ViewerCommandError> {
        let mut sessions = self
            .sessions
            .lock()
            .map_err(|_| ViewerCommandError::Unavailable)?;
        let session = checked_session_mut(&mut sessions, viewer_label)?;
        Ok(LocalViewerContentResponse {
            tokenized: session.tokenized.clone(),
            references: session.references.iter().cloned().collect(),
        })
    }

    async fn reveal_for_viewer(
        &self,
        viewer_label: &str,
        token: &str,
    ) -> Result<Zeroizing<String>, ViewerCommandError> {
        let redactor = self
            .redactor
            .as_ref()
            .ok_or(ViewerCommandError::Unavailable)?;
        let mut capability = {
            let mut sessions = self
                .sessions
                .lock()
                .map_err(|_| ViewerCommandError::Unavailable)?;
            let session = checked_session_mut(&mut sessions, viewer_label)?;
            if !session
                .references
                .iter()
                .any(|reference| reference.token == token)
            {
                return Err(ViewerCommandError::TokenNotInSession);
            }
            if !session.consumed_tokens.insert(token.to_owned()) {
                return Err(ViewerCommandError::Replayed);
            }
            ViewerCapability::issue(viewer_label, token, session.expires_at)
        };
        let revealed = redactor
            .reveal(token, viewer_label, &mut capability)
            .map_err(|_| ViewerCommandError::Unavailable)?;
        let class = token
            .strip_prefix('[')
            .and_then(|body| body.split_once(':'))
            .map(|(category, _)| category)
            .unwrap_or("unknown");
        let audit_input = content_free_audit_input(RedactionAction::Reveal, class, &revealed)?;
        self.append_audit_input(audit_input).await?;
        Ok(revealed)
    }

    fn close_session(&self, viewer_label: &str) {
        if let Ok(mut sessions) = self.sessions.lock() {
            sessions.remove(viewer_label);
        }
    }

    async fn pseudonymize_selection(
        &self,
        request: ManualPseudonymRequest,
    ) -> Result<TokenizedText, ViewerCommandError> {
        let value = Zeroizing::new(request.value);
        if value.trim().is_empty() {
            return Err(ViewerCommandError::EmptyValue);
        }
        let category = parse_manual_category(&request.category)?;
        let redactor = self
            .redactor
            .as_ref()
            .ok_or(ViewerCommandError::Unavailable)?;
        let token = redactor
            .pseudonymiser
            .token(&category, value.as_str())
            .map_err(|_| ViewerCommandError::InvalidCategory)?;
        let audit_input = content_free_audit_input(
            RedactionAction::Pseudonymize,
            &category.to_string().to_uppercase(),
            &value,
        )?;
        drop(value);
        self.append_audit_input(audit_input).await?;
        let reference =
            OpaqueTokenReference::new(token.clone(), category.to_string().to_uppercase());
        Ok(TokenizedText::new(token, vec![reference]))
    }

    async fn append_audit_input(&self, input: AuditEntryInput) -> Result<(), ViewerCommandError> {
        let store = self
            .audit_store
            .as_ref()
            .ok_or(ViewerCommandError::Unavailable)?;
        store
            .append(&self.audit_tenant, vec![input])
            .await
            .map_err(|_| ViewerCommandError::Unavailable)?;
        Ok(())
    }
}

fn content_free_audit_input(
    action: RedactionAction,
    category: &str,
    value: &str,
) -> Result<AuditEntryInput, ViewerCommandError> {
    let span_hash = blake3::hash(value.as_bytes()).to_hex().to_string();
    let mut id_nonce = [0_u8; 16];
    getrandom::fill(&mut id_nonce).map_err(|_| ViewerCommandError::Unavailable)?;
    let id = blake3::hash(&id_nonce).to_hex().to_string();
    Ok(AuditEntryInput {
        id,
        category: category.to_owned(),
        action,
        span_hash,
        span_length: u32::try_from(value.len()).unwrap_or(u32::MAX),
        confidence: None,
        source: EntitySource::Manual,
        pipeline_version: "hacienda-secrecy-client-manual-v1".to_owned(),
        config_hash: LOCAL_AUDIT_CONFIG_HASH.to_owned(),
        principal: None,
        vertical: None,
        model: None,
    })
}

const LOCAL_AUDIT_CONFIG_HASH: &str = "hacienda-secrecy-client-local-audit-v1";

struct ValidatedViewerPayload {
    tokenized: TokenizedText,
    references: HashSet<OpaqueTokenReference>,
}

fn validate_viewer_payload(
    redactor: &HaciendaLocalRedactor,
    request: OpenLocalViewerRequest,
) -> Result<ValidatedViewerPayload, ViewerCommandError> {
    let sanitized = redactor
        .tokenize(&request.tokenized.text)
        .map_err(|_| ViewerCommandError::InvalidTokenizedPayload)?;
    let supplied: HashSet<_> = request.references.iter().cloned().collect();
    let annotated: HashSet<_> = request.tokenized.references.iter().cloned().collect();
    let authenticated: HashSet<_> = sanitized.references.iter().cloned().collect();
    if sanitized.text != request.tokenized.text
        || supplied.len() != request.references.len()
        || annotated.len() != request.tokenized.references.len()
        || supplied != annotated
        || supplied != authenticated
    {
        return Err(ViewerCommandError::InvalidTokenizedPayload);
    }
    Ok(ValidatedViewerPayload {
        tokenized: request.tokenized,
        references: supplied,
    })
}

fn checked_session_mut<'a>(
    sessions: &'a mut HashMap<String, ViewerSession>,
    viewer_label: &str,
) -> Result<&'a mut ViewerSession, ViewerCommandError> {
    if !viewer_label.starts_with(LOCAL_VIEWER_PREFIX) {
        return Err(ViewerCommandError::Unauthorized);
    }
    let expired = sessions
        .get(viewer_label)
        .map(|session| session.expires_at <= Instant::now())
        .unwrap_or(false);
    if expired {
        sessions.remove(viewer_label);
        return Err(ViewerCommandError::Expired);
    }
    sessions
        .get_mut(viewer_label)
        .ok_or(ViewerCommandError::Unauthorized)
}

fn new_viewer_label() -> Result<String, ViewerCommandError> {
    let mut nonce = [0_u8; 32];
    getrandom::fill(&mut nonce).map_err(|_| ViewerCommandError::Unavailable)?;
    let mut label = String::with_capacity(LOCAL_VIEWER_PREFIX.len() + nonce.len() * 2);
    label.push_str(LOCAL_VIEWER_PREFIX);
    for byte in nonce {
        use std::fmt::Write as _;
        write!(label, "{byte:02x}").map_err(|_| ViewerCommandError::Unavailable)?;
    }
    Ok(label)
}

#[derive(Deserialize)]
struct ManualPseudonymRequest {
    category: String,
    value: String,
}

fn parse_manual_category(value: &str) -> Result<PiiCategory, ViewerCommandError> {
    if let Some(custom) = value.strip_prefix("custom:") {
        let valid = !custom.is_empty()
            && custom.len() <= 48
            && custom.bytes().enumerate().all(|(index, byte)| {
                byte.is_ascii_lowercase()
                    || byte.is_ascii_digit() && index > 0
                    || byte == b'_' && index > 0
            });
        return valid
            .then(|| PiiCategory::Custom(custom.to_owned()))
            .ok_or(ViewerCommandError::InvalidCategory);
    }
    serde_json::from_value(serde_json::Value::String(value.to_ascii_lowercase()))
        .map_err(|_| ViewerCommandError::InvalidCategory)
}

impl RedactionAdapter for HaciendaLocalRedactor {
    fn redact(&self, request: &ProviderRequest) -> Result<RedactedProviderRequest, RedactionError> {
        let (provider, conversation_id, context_id, prompt, context) = match request {
            ProviderRequest::ClaudeDesktop(request) => (
                ProviderKind::ClaudeDesktop,
                request.conversation_id.clone(),
                request.context_id.clone(),
                request.prompt.as_str(),
                request.context.as_slice(),
            ),
            ProviderRequest::Codex(request) => (
                ProviderKind::Codex,
                request.conversation_id.clone(),
                request.context_id.clone(),
                request.prompt.as_str(),
                request.context.as_slice(),
            ),
        };
        Ok(RedactedProviderRequest {
            provider,
            conversation_id,
            context_id,
            prompt: self.tokenize(prompt)?,
            context: context
                .iter()
                .map(|item| {
                    Ok(RedactedContext {
                        label: self.tokenize(&item.label)?.text,
                        content: self.tokenize(&item.content)?,
                    })
                })
                .collect::<Result<_, RedactionError>>()?,
        })
    }

    fn redact_text(&self, text: &str) -> Result<TokenizedText, RedactionError> {
        self.tokenize(text)
    }
}

struct SharedHaciendaRedactor(Arc<HaciendaLocalRedactor>);

impl RedactionAdapter for SharedHaciendaRedactor {
    fn redact(&self, request: &ProviderRequest) -> Result<RedactedProviderRequest, RedactionError> {
        self.0.redact(request)
    }

    fn redact_text(&self, text: &str) -> Result<TokenizedText, RedactionError> {
        self.0.redact_text(text)
    }
}

impl From<&OpenOriginalRequest> for OriginalBinding {
    fn from(request: &OpenOriginalRequest) -> Self {
        Self {
            document_ref: request.document_ref.clone(),
            version_ref: request.version_ref.clone(),
            original_ref: request.original_ref.clone(),
            viewer_ref: request.viewer_ref.clone(),
            capability_ref: request.capability_ref.clone(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum OriginalViewerError {
    Unavailable,
    InvalidRequest,
    Unauthorized,
    OpenFailed,
}

impl std::fmt::Display for OriginalViewerError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::Unavailable => {
                "original file opening is unavailable until native picker ingestion is configured"
            }
            Self::InvalidRequest => "original viewer request is invalid",
            Self::Unauthorized => "original viewer capability is unavailable",
            Self::OpenFailed => "original viewer could not open the document",
        })
    }
}

trait DesktopOpener: Send + Sync {
    fn open(&self, path: &Path) -> Result<(), ()>;
}

struct SystemDesktopOpener;

impl DesktopOpener for SystemDesktopOpener {
    fn open(&self, path: &Path) -> Result<(), ()> {
        #[cfg(target_os = "macos")]
        let status = std::process::Command::new("open").arg(path).status();
        #[cfg(target_os = "linux")]
        let status = std::process::Command::new("xdg-open").arg(path).status();
        #[cfg(target_os = "windows")]
        let status = std::process::Command::new("cmd")
            .args(["/C", "start", ""])
            .arg(path)
            .status();
        status
            .map(|value| value.success())
            .map_err(|_| ())?
            .then_some(())
            .ok_or(())
    }
}

struct OriginalBindingEntry {
    path: PathBuf,
    expires_at: Instant,
    consumed: bool,
}

struct NativeOriginalViewer {
    bindings: Mutex<HashMap<OriginalBinding, OriginalBindingEntry>>,
    opener: Arc<dyn DesktopOpener>,
}

impl NativeOriginalViewer {
    fn new(opener: Arc<dyn DesktopOpener>) -> Self {
        Self {
            bindings: Mutex::new(HashMap::new()),
            opener,
        }
    }

    fn register(&self, binding: OriginalBinding, path: PathBuf) {
        self.bindings.lock().expect("binding lock").insert(
            binding,
            OriginalBindingEntry {
                path,
                expires_at: Instant::now() + Duration::from_secs(300),
                consumed: false,
            },
        );
    }
}

trait OriginalViewerAdapter: Send + Sync {
    fn authorize_original(&self, request: &OpenOriginalRequest) -> Result<(), OriginalViewerError>;
    fn open_original(&self, request: &OpenOriginalRequest) -> Result<(), OriginalViewerError>;
    fn register_original(&self, binding: OriginalBinding, path: PathBuf);
}

impl OriginalViewerAdapter for NativeOriginalViewer {
    fn authorize_original(&self, request: &OpenOriginalRequest) -> Result<(), OriginalViewerError> {
        let refs = [
            &request.document_ref,
            &request.version_ref,
            &request.original_ref,
            &request.viewer_ref,
            &request.capability_ref,
        ];
        if refs.iter().any(|reference| reference.trim().is_empty()) {
            return Err(OriginalViewerError::InvalidRequest);
        }
        let binding = OriginalBinding::from(request);
        let bindings = self
            .bindings
            .lock()
            .map_err(|_| OriginalViewerError::Unavailable)?;
        let entry = bindings
            .get(&binding)
            .filter(|entry| !entry.consumed && entry.expires_at > Instant::now())
            .ok_or(OriginalViewerError::Unauthorized)?;
        if !entry.path.is_file() {
            return Err(OriginalViewerError::Unavailable);
        }
        Ok(())
    }

    fn open_original(&self, request: &OpenOriginalRequest) -> Result<(), OriginalViewerError> {
        let refs = [
            &request.document_ref,
            &request.version_ref,
            &request.original_ref,
            &request.viewer_ref,
            &request.capability_ref,
        ];
        if refs.iter().any(|reference| reference.trim().is_empty()) {
            return Err(OriginalViewerError::InvalidRequest);
        }
        let binding = OriginalBinding::from(request);
        let mut bindings = self
            .bindings
            .lock()
            .map_err(|_| OriginalViewerError::Unavailable)?;
        let invalid = bindings
            .get(&binding)
            .map(|entry| entry.consumed || entry.expires_at <= Instant::now())
            .unwrap_or(true);
        if invalid {
            bindings.remove(&binding);
            return Err(OriginalViewerError::Unauthorized);
        }
        let entry = bindings
            .get_mut(&binding)
            .ok_or(OriginalViewerError::Unauthorized)?;
        let path = entry.path.clone();
        if !path.is_file() {
            return Err(OriginalViewerError::Unavailable);
        }
        entry.consumed = true;
        drop(bindings);
        self.opener
            .open(&path)
            .map_err(|_| OriginalViewerError::OpenFailed)?;
        Ok(())
    }

    fn register_original(&self, binding: OriginalBinding, path: PathBuf) {
        self.register(binding, path);
    }
}

struct FailClosedOriginalViewer;

impl OriginalViewerAdapter for FailClosedOriginalViewer {
    fn authorize_original(
        &self,
        _request: &OpenOriginalRequest,
    ) -> Result<(), OriginalViewerError> {
        Err(OriginalViewerError::Unavailable)
    }

    fn open_original(&self, _request: &OpenOriginalRequest) -> Result<(), OriginalViewerError> {
        Err(OriginalViewerError::Unavailable)
    }

    fn register_original(&self, _binding: OriginalBinding, _path: PathBuf) {}
}

struct OriginalViewerCommandState {
    adapter: Arc<dyn OriginalViewerAdapter>,
    audit_store: Option<Arc<dyn AuditStore>>,
}

impl OriginalViewerCommandState {
    fn fail_closed() -> Self {
        Self {
            adapter: Arc::new(FailClosedOriginalViewer),
            audit_store: None,
        }
    }

    fn native(audit_store: Arc<dyn AuditStore>) -> Self {
        Self {
            adapter: Arc::new(NativeOriginalViewer::new(Arc::new(SystemDesktopOpener))),
            audit_store: Some(audit_store),
        }
    }

    fn register_original(&self, binding: OriginalBinding, path: PathBuf) {
        self.adapter.register_original(binding, path);
    }
}

#[tauri::command(rename = "open-original")]
async fn open_original(
    window: WebviewWindow,
    state: tauri::State<'_, OriginalViewerCommandState>,
    request: OpenOriginalRequest,
) -> Result<OpenOriginalResponse, String> {
    require_main_window(&window)?;
    let adapter = Arc::clone(&state.adapter);
    let authorization = OpenOriginalRequest {
        document_ref: request.document_ref.clone(),
        version_ref: request.version_ref.clone(),
        original_ref: request.original_ref.clone(),
        viewer_ref: request.viewer_ref.clone(),
        capability_ref: request.capability_ref.clone(),
    };
    let check_adapter = Arc::clone(&adapter);
    tokio::task::spawn_blocking(move || check_adapter.authorize_original(&authorization))
        .await
        .map_err(|_| "Original viewer authorization task failed".to_owned())?
        .map_err(|error| error.to_string())?;
    let audit = state
        .audit_store
        .as_ref()
        .ok_or_else(|| "Original viewer audit failed closed".to_owned())?;
    let digest = blake3::hash(
        format!(
            "{}:{}:{}:{}:{}",
            request.document_ref,
            request.version_ref,
            request.original_ref,
            request.viewer_ref,
            request.capability_ref
        )
        .as_bytes(),
    )
    .to_hex()
    .to_string();
    audit
        .append(
            &TenantId::new("local-full"),
            vec![AuditEntryInput {
                id: opaque_reference("audit"),
                category: "original_viewer".into(),
                action: RedactionAction::Reveal,
                span_hash: digest,
                span_length: 0,
                confidence: None,
                source: EntitySource::Manual,
                pipeline_version: "hacienda-secrecy-client-viewer-v1".into(),
                config_hash: LOCAL_AUDIT_CONFIG_HASH.into(),
                principal: None,
                vertical: None,
                model: None,
            }],
        )
        .await
        .map_err(|_| "Original viewer audit failed closed".to_owned())?;
    tokio::task::spawn_blocking(move || adapter.open_original(&request))
        .await
        .map_err(|_| "Original viewer launch task failed".to_owned())?
        .map_err(|error| error.to_string())?;
    Ok(OpenOriginalResponse { opened: true })
}

/// Tauri state for the transient provider path. Both adapters are injected so
/// production can use a loopback gateway while tests can use a memory seam.
#[derive(Clone)]
pub struct ProviderCommandState {
    transport: Arc<Mutex<Box<dyn ProviderTransport + Send>>>,
    redactor: Arc<Mutex<Option<Box<dyn RedactionAdapter + Send + Sync>>>>,
    pii_detection_ready: bool,
}

impl ProviderCommandState {
    /// Build state with an explicit redaction capability and provider transport.
    /// Omitting the redactor is intentional and fails closed on every request.
    pub fn new<T, R>(transport: T, redactor: Option<R>) -> Self
    where
        T: ProviderTransport + Send + 'static,
        R: RedactionAdapter + Send + Sync + 'static,
    {
        Self {
            transport: Arc::new(Mutex::new(Box::new(transport))),
            redactor: Arc::new(Mutex::new(redactor.map(|redactor| {
                Box::new(redactor) as Box<dyn RedactionAdapter + Send + Sync>
            }))),
            pii_detection_ready: false,
        }
    }

    fn with_complete_pii_detector(mut self) -> Self {
        self.pii_detection_ready = true;
        self
    }

    /// Construct the default desktop state. No provider call is possible until
    /// a real redactor and gateway transport are injected by the host.
    pub fn fail_closed() -> Self {
        Self::new(FailClosedTransport, None::<FailClosedRedactor>)
    }

    /// Build the production path from gateway environment configuration.
    /// Missing or malformed configuration intentionally returns an error so the
    /// caller can install the fail-closed state instead.
    pub fn from_environment() -> Result<Self, String> {
        let redactor = Arc::new(
            HaciendaLocalRedactor::new()
                .map_err(|_| "Hacienda local redactor is unavailable".to_owned())?,
        );
        Self::from_environment_with_redactor(Some(redactor))
    }

    fn from_environment_with_redactor(
        redactor: Option<Arc<HaciendaLocalRedactor>>,
    ) -> Result<Self, String> {
        let installation = installation_config_from_environment()?;
        let gateway = provider_gateway_for_installation(&installation)?;
        let model = env::var("HACIENDA_PROVIDER_MODEL").unwrap_or_else(|_| "hacienda-local".into());
        let max_tokens = env::var("HACIENDA_PROVIDER_MAX_TOKENS")
            .unwrap_or_else(|_| "2048".into())
            .parse::<u32>()
            .map_err(|_| "Hacienda provider token limit is invalid".to_owned())?;
        let credential = gateway_credential(&gateway.authentication)?;
        let transport =
            GatewayProviderTransport::new(hacienda_client_core::ProviderTransportConfig::new(
                gateway.endpoint,
                model,
                max_tokens,
                credential,
            ))
            .map_err(|_| "Hacienda gateway transport is invalid".to_owned())?;
        let redactor =
            redactor.ok_or_else(|| "Hacienda local redactor is unavailable".to_owned())?;
        if !redactor.pii_detection_complete {
            return Err("complete local PII detector is unavailable".to_owned());
        }
        Ok(Self::new(transport, Some(SharedHaciendaRedactor(redactor)))
            .with_complete_pii_detector())
    }

    /// Install the configured runtime when possible, otherwise preserve the
    /// no-egress fail-closed contract.
    pub fn runtime() -> Self {
        Self::from_environment().unwrap_or_else(|_| Self::fail_closed())
    }

    fn runtime_with_viewer(app_data_dir: Option<&Path>) -> (Self, LocalViewerState) {
        let redactor = app_data_dir
            .map(HaciendaLocalRedactor::new_with_app_data_dir)
            .unwrap_or_else(HaciendaLocalRedactor::new)
            .ok()
            .map(Arc::new);
        let viewer = LocalViewerState::new(redactor.clone());
        let provider =
            Self::from_environment_with_redactor(redactor).unwrap_or_else(|_| Self::fail_closed());
        (provider, viewer)
    }

    /// Redact first, then stream through the injected token-only transport.
    pub fn stream(&self, request: &ProviderRequest) -> Result<ProviderResponse, ClientError> {
        if !self.pii_detection_ready {
            return Err(ClientError::RedactionUnavailable);
        }
        let safe_request = {
            let redactor = self
                .redactor
                .lock()
                .map_err(|_| ClientError::RedactionUnavailable)?;
            let redactor = redactor.as_ref().ok_or(ClientError::RedactionUnavailable)?;
            redactor.redact(request)?
        };
        let response = self
            .transport
            .lock()
            .map_err(|_| ClientError::Transport(TransportError::Failed))?
            .stream(&safe_request)?;
        let response = {
            let redactor = self
                .redactor
                .lock()
                .map_err(|_| ClientError::RedactionUnavailable)?;
            let redactor = redactor.as_ref().ok_or(ClientError::RedactionUnavailable)?;
            redact_response(redactor.as_ref(), response)?
        };
        validate_response(request, &response)?;
        Ok(response)
    }
}

const INSTALLATION_CONFIG_ENV: &str = "HACIENDA_INSTALLATION_CONFIG";
const GATEWAY_TOKEN_ENV: &str = "HACIENDA_GATEWAY_TOKEN";
const PROVIDER_CREDENTIAL_ENV: &str = "HACIENDA_PROVIDER_CREDENTIAL";

struct ProviderGateway {
    endpoint: String,
    authentication: Option<SecretReference>,
}

fn installation_config_from_environment() -> Result<InstallationConfig, String> {
    let serialized = env::var(INSTALLATION_CONFIG_ENV)
        .map_err(|_| "Hacienda installation configuration is not configured".to_owned())?;
    let config: InstallationConfig = serde_json::from_str(&serialized)
        .map_err(|_| "Hacienda installation configuration is invalid".to_owned())?;
    config
        .validate()
        .map_err(|_| "Hacienda installation configuration is invalid".to_owned())?;
    Ok(config)
}

fn provider_gateway_for_installation(
    installation: &InstallationConfig,
) -> Result<ProviderGateway, String> {
    // Validate here as well as at the environment boundary so this helper cannot
    // accidentally become a route around the profile's topology invariants.
    installation
        .validate()
        .map_err(|_| "Hacienda installation configuration is invalid".to_owned())?;
    match installation {
        InstallationConfig::LocalFull(config) => match config.gateway.as_ref() {
            Some(GatewayConfig::Local { endpoint, .. }) => Ok(ProviderGateway {
                endpoint: endpoint.0.clone(),
                authentication: None,
            }),
            _ => Err("Hacienda LocalFull gateway is invalid".to_owned()),
        },
        InstallationConfig::TeamOnPremise(config) => match config.gateway.as_ref() {
            Some(GatewayConfig::AuthenticatedTenantScoped {
                endpoint,
                authentication,
                ..
            }) => Ok(ProviderGateway {
                endpoint: endpoint.0.clone(),
                authentication: Some(authentication.clone()),
            }),
            _ => Err("Hacienda TeamOnPremise gateway is invalid".to_owned()),
        },
    }
}

fn gateway_credential(authentication: &Option<SecretReference>) -> Result<String, String> {
    match authentication {
        // LocalFull's loopback gateway may still require its configured transport
        // credential, but it has no profile SecretReference to resolve.
        None => env::var(GATEWAY_TOKEN_ENV)
            .or_else(|_| env::var(PROVIDER_CREDENTIAL_ENV))
            .map_err(|_| "Hacienda gateway credential is not configured".to_owned()),
        Some(reference) => {
            let variable = secret_reference_environment_name(reference)?;
            env::var(&variable)
                .map_err(|_| "Hacienda gateway credential is not configured".to_owned())
        }
    }
}

fn secret_reference_environment_name(reference: &SecretReference) -> Result<String, String> {
    let value = reference.0.trim();
    if let Some(name) = value.strip_prefix("env:") {
        if name.is_empty()
            || !name.bytes().enumerate().all(|(index, byte)| {
                byte.is_ascii_uppercase() || byte.is_ascii_digit() && index > 0 || byte == b'_'
            })
        {
            return Err("Hacienda gateway credential reference is invalid".to_owned());
        }
        return Ok(name.to_owned());
    }
    if value.is_empty() {
        return Err("Hacienda gateway credential reference is invalid".to_owned());
    }
    let normalized = value
        .bytes()
        .map(|byte| {
            if byte.is_ascii_uppercase() || byte.is_ascii_digit() {
                byte
            } else if byte.is_ascii_lowercase() {
                byte.to_ascii_uppercase()
            } else {
                b'_'
            }
        })
        .collect::<Vec<_>>();
    Ok(format!(
        "HACIENDA_CREDENTIAL_{}",
        String::from_utf8(normalized)
            .map_err(|_| "Hacienda gateway credential reference is invalid")?
    ))
}

fn redact_response(
    redactor: &dyn RedactionAdapter,
    response: ProviderResponse,
) -> Result<ProviderResponse, ClientError> {
    let redact = |mut chunk: ProviderStreamChunk| {
        let existing = chunk.content.references.clone();
        let mut content = redactor.redact_text(&chunk.content.text)?;
        for reference in existing {
            if !content.references.contains(&reference) {
                content.references.push(reference);
            }
        }
        chunk.content = content;
        Ok(chunk)
    };
    Ok(match response {
        ProviderResponse::ClaudeDesktop(mut response) => {
            response.chunks = response
                .chunks
                .into_iter()
                .map(redact)
                .collect::<Result<_, ClientError>>()?;
            ProviderResponse::ClaudeDesktop(response)
        }
        ProviderResponse::Codex(mut response) => {
            response.chunks = response
                .chunks
                .into_iter()
                .map(redact)
                .collect::<Result<_, ClientError>>()?;
            ProviderResponse::Codex(response)
        }
    })
}

struct FailClosedRedactor;

impl RedactionAdapter for FailClosedRedactor {
    fn redact(
        &self,
        _request: &ProviderRequest,
    ) -> Result<RedactedProviderRequest, hacienda_client_core::RedactionError> {
        Err(hacienda_client_core::RedactionError::Rejected)
    }
}

struct FailClosedTransport;

impl ProviderTransport for FailClosedTransport {
    fn stream(
        &mut self,
        _request: &RedactedProviderRequest,
    ) -> Result<ProviderResponse, TransportError> {
        Err(TransportError::Failed)
    }
}

fn validate_response(
    request: &ProviderRequest,
    response: &ProviderResponse,
) -> Result<(), ClientError> {
    let (provider, conversation_id, context_id, chunks) = match response {
        ProviderResponse::ClaudeDesktop(response) => (
            hacienda_client_core::ProviderKind::ClaudeDesktop,
            &response.conversation_id,
            &response.context_id,
            &response.chunks,
        ),
        ProviderResponse::Codex(response) => (
            hacienda_client_core::ProviderKind::Codex,
            &response.conversation_id,
            &response.context_id,
            &response.chunks,
        ),
    };
    if provider != request.provider()
        || conversation_id != request.conversation_id()
        || context_id != request.context_id()
        || chunks.iter().any(|chunk: &ProviderStreamChunk| {
            chunk.conversation_id.as_str() != request.conversation_id().as_str()
                || chunk.context_id.as_str() != request.context_id().as_str()
        })
    {
        return Err(ClientError::Transport(TransportError::Failed));
    }
    Ok(())
}

#[tauri::command(rename = "stream-claude")]
async fn stream_claude(
    window: WebviewWindow,
    state: tauri::State<'_, ProviderCommandState>,
    request: ClaudeDesktopRequest,
) -> Result<ClaudeDesktopResponse, ClientError> {
    if window.label() != MAIN_WINDOW_LABEL {
        return Err(ClientError::RedactionUnavailable);
    }
    let provider = state.inner().clone();
    let response = tokio::task::spawn_blocking(move || {
        provider.stream(&ProviderRequest::ClaudeDesktop(request))
    })
    .await
    .map_err(|_| ClientError::Transport(TransportError::Failed))??;
    match response {
        ProviderResponse::ClaudeDesktop(response) => Ok(response),
        ProviderResponse::Codex(_) => Err(ClientError::Transport(TransportError::Failed)),
    }
}

#[tauri::command(rename = "stream-codex")]
async fn stream_codex(
    window: WebviewWindow,
    state: tauri::State<'_, ProviderCommandState>,
    request: CodexRequest,
) -> Result<CodexResponse, ClientError> {
    if window.label() != MAIN_WINDOW_LABEL {
        return Err(ClientError::RedactionUnavailable);
    }
    let provider = state.inner().clone();
    let response =
        tokio::task::spawn_blocking(move || provider.stream(&ProviderRequest::Codex(request)))
            .await
            .map_err(|_| ClientError::Transport(TransportError::Failed))??;
    match response {
        ProviderResponse::Codex(response) => Ok(response),
        ProviderResponse::ClaudeDesktop(_) => Err(ClientError::Transport(TransportError::Failed)),
    }
}

/// Opens a local-only viewer from already-tokenized content. The caller cannot supply
/// an original document or a reveal capability; the session is bound to the new window.
#[tauri::command(rename = "open-local-viewer")]
async fn open_local_viewer(
    window: WebviewWindow,
    app: tauri::AppHandle,
    state: tauri::State<'_, LocalViewerState>,
    request: OpenLocalViewerRequest,
) -> Result<OpenLocalViewerResponse, ViewerCommandError> {
    let viewer_label = state.open_session(window.label(), request)?;
    let builder = WebviewWindowBuilder::new(
        &app,
        viewer_label.clone(),
        WebviewUrl::App("index.html?viewer=local".into()),
    )
    .title("Hacienda Viewer");
    if builder.build().is_err() {
        state.close_session(&viewer_label);
        return Err(ViewerCommandError::WindowOpenFailed);
    }
    Ok(OpenLocalViewerResponse { opened: true })
}

/// Returns the token-only payload bound to the calling viewer window.
#[tauri::command(rename = "local-viewer-content")]
fn local_viewer_content(
    window: WebviewWindow,
    state: tauri::State<'_, LocalViewerState>,
) -> Result<LocalViewerContentResponse, ViewerCommandError> {
    state.content_for_viewer(window.label())
}

/// Reveals one token owned by the calling viewer window's live session.
#[tauri::command(rename = "reveal-local-token")]
async fn reveal_local_token(
    window: WebviewWindow,
    state: tauri::State<'_, LocalViewerState>,
    token: String,
) -> Result<String, ViewerCommandError> {
    state
        .reveal_for_viewer(window.label(), &token)
        .await
        .map(|revealed| revealed.to_string())
}

/// Tiptap's manual-sensitive action passes the selected value only across this local
/// IPC seam. The response contains the Hacienda token, never the submitted value.
#[tauri::command(rename = "pseudonymize-selection")]
async fn pseudonymize_selection(
    window: WebviewWindow,
    state: tauri::State<'_, LocalViewerState>,
    request: ManualPseudonymRequest,
) -> Result<TokenizedText, ViewerCommandError> {
    if window.label() != MAIN_WINDOW_LABEL {
        return Err(ViewerCommandError::Unauthorized);
    }
    state.pseudonymize_selection(request).await
}

/// Reports whether this build/runtime has a complete local detector for provider egress.
#[tauri::command(rename = "local-pii-status")]
fn local_pii_status(
    viewer: tauri::State<'_, LocalViewerState>,
    provider: tauri::State<'_, ProviderCommandState>,
) -> LocalPiiStatusResponse {
    viewer.pii_status(provider.pii_detection_ready)
}

/// The picker runs in the native process. Only the redacted projection crosses IPC;
/// the selected path and bytes remain inside the LocalFull runtime.
#[tauri::command(rename = "pick-document")]
async fn pick_document(
    window: WebviewWindow,
    state: tauri::State<'_, LocalFullDocumentState>,
    originals: tauri::State<'_, OriginalViewerCommandState>,
) -> Result<DocumentProjectionResponse, String> {
    require_main_window(&window)?;
    let Some(file) = rfd::AsyncFileDialog::new().pick_file().await else {
        return Err("Document selection was cancelled".to_owned());
    };
    let path = file.path().to_path_buf();
    let mut service = state.service().await?;
    let service = service
        .as_mut()
        .ok_or_else(|| "LocalFull document service is unavailable".to_owned())?;
    let (projection, binding, original_path) = service.ingest_path(path).await?;
    originals.register_original(binding, original_path);
    Ok(projection)
}

#[derive(Debug, Serialize)]
struct LocalEmbeddingStatusResponse {
    ready: bool,
    model: &'static str,
}

/// Explicitly downloads or loads the pinned Xberg multilingual embedding model.
/// Document text is never passed to this model-installation operation.
#[tauri::command(rename = "install-local-rag-model")]
async fn install_local_rag_model(
    window: WebviewWindow,
    state: tauri::State<'_, LocalFullDocumentState>,
) -> Result<LocalEmbeddingStatusResponse, String> {
    require_main_window(&window)?;
    let mut service = state.service().await?;
    let service = service
        .as_mut()
        .ok_or_else(|| "LocalFull document service is unavailable".to_owned())?;
    service.install_embedding_model().await?;
    Ok(LocalEmbeddingStatusResponse {
        ready: service.embedding_ready,
        model: "xberg:preset:multilingual",
    })
}

/// Redacts a possibly sensitive query locally, embeds only its tokenized form,
/// and returns token-only hybrid-search snippets for the renderer.
#[tauri::command(rename = "search-local-documents")]
async fn search_local_documents(
    window: WebviewWindow,
    state: tauri::State<'_, LocalFullDocumentState>,
    request: LocalSearchRequest,
) -> Result<LocalSearchResponse, String> {
    require_main_window(&window)?;
    let service = state.service().await?;
    service
        .as_ref()
        .ok_or_else(|| "LocalFull document service is unavailable".to_owned())?
        .search(request)
        .await
}

#[tauri::command(rename = "load-document")]
async fn load_document(
    window: WebviewWindow,
    state: tauri::State<'_, LocalFullDocumentState>,
    request: LoadDocumentRequest,
) -> Result<DocumentProjectionResponse, String> {
    require_main_window(&window)?;
    let service = state.service().await?;
    service
        .as_ref()
        .ok_or_else(|| "LocalFull document service is unavailable".to_owned())?
        .load(&request)
}

#[tauri::command(rename = "save-document")]
async fn save_document(
    window: WebviewWindow,
    state: tauri::State<'_, LocalFullDocumentState>,
    originals: tauri::State<'_, OriginalViewerCommandState>,
    request: SaveDocumentRequest,
) -> Result<SaveDocumentResponse, String> {
    require_main_window(&window)?;
    let mut service = state.service().await?;
    let service = service
        .as_mut()
        .ok_or_else(|| "LocalFull document service is unavailable".to_owned())?;
    let document_ref = request.document_ref.clone();
    let saved = service.save(request).await?;
    let (binding, path) = service.binding_for(&document_ref)?;
    originals.register_original(binding, path);
    Ok(saved)
}

#[cfg(all(
    not(test),
    any(
        all(
            target_os = "macos",
            any(target_arch = "x86_64", target_arch = "aarch64")
        ),
        all(
            target_os = "linux",
            any(target_arch = "x86_64", target_arch = "aarch64")
        ),
        all(
            target_os = "windows",
            any(target_arch = "x86_64", target_arch = "aarch64")
        )
    )
))]
fn configure_bundled_onnxruntime(app: &tauri::App) {
    let library = app.path().resource_dir().ok().map(|resource_dir| {
        resource_dir
            .join("resources/onnxruntime")
            .join(ONNXRUNTIME_RESOURCE_TARGET)
            .join("lib")
            .join(ONNXRUNTIME_LIBRARY_FILE)
    });
    if let Some(library) = library.filter(|path| path.is_file()) {
        // This runs during Tauri setup, before any local model is initialized.
        env::set_var("ORT_DYLIB_PATH", library);
    }
}

#[cfg(all(
    not(test),
    not(any(
        all(
            target_os = "macos",
            any(target_arch = "x86_64", target_arch = "aarch64")
        ),
        all(
            target_os = "linux",
            any(target_arch = "x86_64", target_arch = "aarch64")
        ),
        all(
            target_os = "windows",
            any(target_arch = "x86_64", target_arch = "aarch64")
        )
    ))
))]
fn configure_bundled_onnxruntime(_app: &tauri::App) {}

#[cfg(not(test))]
fn main() {
    tauri::Builder::default()
        .setup(|app| {
            configure_bundled_onnxruntime(app);
            let app_data_dir = app
                .path()
                .app_local_data_dir()
                .map_err(|_| "LocalFull storage path is unavailable")?;
            let (provider_state, viewer_state) =
                ProviderCommandState::runtime_with_viewer(Some(&app_data_dir));
            app.manage(provider_state);
            let audit_store = app
                .path()
                .app_local_data_dir()
                .ok()
                .and_then(|root| {
                    FileAuditStore::open(
                        root.join("audit"),
                        NodeId::default(),
                        LOCAL_AUDIT_CONFIG_HASH,
                    )
                    .ok()
                })
                .map(|store| Arc::new(store) as Arc<dyn AuditStore>);
            let viewer_state = match &audit_store {
                Some(store) => viewer_state.with_audit_store(Arc::clone(store)),
                None => viewer_state,
            };
            app.manage(viewer_state);
            app.manage(LocalFullDocumentState::new(
                app_data_dir,
                audit_store.as_ref().map(Arc::clone),
            ));
            if let Some(store) = audit_store {
                app.manage(OriginalViewerCommandState::native(store));
            } else {
                app.manage(OriginalViewerCommandState::fail_closed());
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            open_original,
            stream_claude,
            stream_codex,
            open_local_viewer,
            local_viewer_content,
            reveal_local_token,
            pseudonymize_selection,
            local_pii_status,
            pick_document,
            install_local_rag_model,
            search_local_documents,
            load_document,
            save_document
        ])
        .run(tauri::generate_context!())
        .expect("failed to run Hacienda Secrecy Client");
}

#[cfg(test)]
fn main() {}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    use xberg::text::ner::NerBackend;
    use xberg::types::entity::{Entity, EntityCategory};
    use xberg::Result as XbergResult;

    struct InjectedNerBackend;

    #[async_trait::async_trait]
    impl NerBackend for InjectedNerBackend {
        async fn detect(
            &self,
            _text: &str,
            _categories: &[EntityCategory],
        ) -> XbergResult<Vec<Entity>> {
            Ok(Vec::new())
        }
    }

    struct RecordingProviderTransport {
        outbound: Arc<Mutex<Option<RedactedProviderRequest>>>,
        response_text: String,
    }

    impl ProviderTransport for RecordingProviderTransport {
        fn stream(
            &mut self,
            request: &RedactedProviderRequest,
        ) -> Result<ProviderResponse, TransportError> {
            *self.outbound.lock().map_err(|_| TransportError::Failed)? = Some(request.clone());
            let chunk = ProviderStreamChunk {
                conversation_id: request.conversation_id.clone(),
                context_id: request.context_id.clone(),
                sequence: 0,
                content: TokenizedText::new(self.response_text.clone(), Vec::new()),
            };
            Ok(match request.provider {
                ProviderKind::ClaudeDesktop => {
                    ProviderResponse::ClaudeDesktop(ClaudeDesktopResponse {
                        conversation_id: request.conversation_id.clone(),
                        context_id: request.context_id.clone(),
                        chunks: vec![chunk],
                    })
                }
                ProviderKind::Codex => ProviderResponse::Codex(CodexResponse {
                    conversation_id: request.conversation_id.clone(),
                    context_id: request.context_id.clone(),
                    chunks: vec![chunk],
                }),
            })
        }
    }

    #[derive(Default)]
    struct TestKeyStore(Mutex<Option<[u8; hacienda_core::redaction::KEY_BYTES]>>);

    impl LocalPseudonymKeyStore for TestKeyStore {
        fn load_or_create(
            &self,
        ) -> Result<Zeroizing<[u8; hacienda_core::redaction::KEY_BYTES]>, VaultError> {
            let mut stored = self.0.lock().map_err(|_| VaultError::Locked)?;
            let key = stored.get_or_insert([7_u8; hacienda_core::redaction::KEY_BYTES]);
            Ok(Zeroizing::new(*key))
        }
    }

    fn local_installation() -> InstallationConfig {
        InstallationConfig::LocalFull(hacienda_client_core::LocalFullConfig {
            client: hacienda_client_core::ClientSurface::LocalTauri,
            trust_boundary: hacienda_client_core::TrustBoundary::LocalProcess,
            hacienda: Some(hacienda_client_core::ServiceEndpoint::new(
                "http://127.0.0.1:8001",
            )),
            xberg: Some(hacienda_client_core::ServiceEndpoint::new(
                "http://localhost:8002",
            )),
            gateway: Some(GatewayConfig::Local {
                id: hacienda_client_core::GatewayId("local-gateway".into()),
                endpoint: hacienda_client_core::ServiceEndpoint::new("http://127.0.0.1:8000"),
            }),
            storage: hacienda_client_core::LocalStorageTopology {
                vault: Some(hacienda_client_core::EncryptedLocalVault {
                    id: hacienda_client_core::VaultId("local-vault".into()),
                    path: hacienda_client_core::StoragePath("./vault".into()),
                    key_reference: SecretReference("env:LOCAL_VAULT_KEY".into()),
                }),
                lancedb: hacienda_client_core::LanceDbTopology::LocalEmbedded {
                    path: hacienda_client_core::StoragePath("./lancedb".into()),
                },
            },
            provider_route: hacienda_client_core::ProviderRoute::RedactedViaGateway {
                gateway: hacienda_client_core::GatewayId("local-gateway".into()),
            },
        })
    }

    fn team_installation() -> InstallationConfig {
        InstallationConfig::TeamOnPremise(hacienda_client_core::TeamOnPremiseConfig {
            client: hacienda_client_core::ClientSurface::LightTauri,
            trust_boundary: hacienda_client_core::TrustBoundary::AuthenticatedTenantGateway,
            gateway: Some(GatewayConfig::AuthenticatedTenantScoped {
                id: hacienda_client_core::GatewayId("team-gateway".into()),
                endpoint: hacienda_client_core::ServiceEndpoint::new(
                    "https://hacienda.example.test/gateway",
                ),
                engine: hacienda_client_core::ServiceEndpoint::new(
                    "https://hacienda.example.test/engine",
                ),
                authentication: SecretReference("oidc:client-session".into()),
                tenant_scope: Some(hacienda_client_core::TenantScope("tenant-a".into())),
            }),
            storage: hacienda_client_core::TeamStorageTopology {
                lancedb: hacienda_client_core::LanceDbTopology::ServerTokenOnly {
                    control_plane: hacienda_client_core::ServiceEndpoint::new(
                        "https://hacienda.example.test/lancedb",
                    ),
                    token_reference: SecretReference("control-plane:lancedb".into()),
                    tenant_scope: Some(hacienda_client_core::TenantScope("tenant-a".into())),
                },
            },
            provider_route: hacienda_client_core::ProviderRoute::RedactedViaGateway {
                gateway: hacienda_client_core::GatewayId("team-gateway".into()),
            },
            tenant_scope: Some(hacienda_client_core::TenantScope("tenant-a".into())),
        })
    }

    #[test]
    fn selects_valid_local_full_loopback_gateway_endpoint() {
        let gateway = provider_gateway_for_installation(&local_installation())
            .expect("valid LocalFull gateway");
        assert_eq!(gateway.endpoint, "http://127.0.0.1:8000");
        assert!(gateway.authentication.is_none());
    }

    #[test]
    fn selects_valid_team_on_premise_https_gateway_and_authentication_reference() {
        let gateway = provider_gateway_for_installation(&team_installation())
            .expect("valid TeamOnPremise gateway");
        assert_eq!(gateway.endpoint, "https://hacienda.example.test/gateway");
        assert_eq!(
            secret_reference_environment_name(
                gateway
                    .authentication
                    .as_ref()
                    .expect("authentication reference"),
            )
            .expect("environment name"),
            "HACIENDA_CREDENTIAL_OIDC_CLIENT_SESSION"
        );
    }

    #[test]
    fn rejects_invalid_profiles_before_gateway_selection() {
        let mut local = local_installation();
        if let InstallationConfig::LocalFull(config) = &mut local {
            config.provider_route = hacienda_client_core::ProviderRoute::DirectUnredacted {
                endpoint: hacienda_client_core::ServiceEndpoint::new(
                    "https://provider.example.test",
                ),
            };
        }
        assert!(provider_gateway_for_installation(&local).is_err());

        let mut team = team_installation();
        if let InstallationConfig::TeamOnPremise(config) = &mut team {
            config.storage.lancedb = hacienda_client_core::LanceDbTopology::DirectClient {
                endpoint: hacienda_client_core::ServiceEndpoint::new(
                    "https://lancedb.example.test",
                ),
            };
        }
        assert!(provider_gateway_for_installation(&team).is_err());
    }

    fn open_viewer_request(tokenized: TokenizedText) -> OpenLocalViewerRequest {
        OpenLocalViewerRequest {
            references: tokenized.references.clone(),
            tokenized,
        }
    }

    fn audited_viewer(redactor: Option<Arc<HaciendaLocalRedactor>>) -> LocalViewerState {
        LocalViewerState::new(redactor).with_audit_store(Arc::new(
            hacienda_core::audit::InMemoryAuditStore::new(LOCAL_AUDIT_CONFIG_HASH),
        ))
    }

    #[test]
    fn local_full_emits_stable_reversible_hacienda_tokens_across_reopen() {
        let store = Arc::new(TestKeyStore::default());
        let first = HaciendaLocalRedactor::from_key_store(store.clone()).expect("vault opens");
        let output = first
            .redact_text("Contact alice@example.com")
            .expect("redacts");
        assert!(output.text.contains("[EMAIL:local:"), "{}", output.text);
        assert!(!output.text.contains("alice@example.com"));
        let token = output.references[0].token.clone();

        let reopened = HaciendaLocalRedactor::from_key_store(store).expect("vault reopens");
        let again = reopened
            .redact_text("Contact alice@example.com")
            .expect("redacts");
        assert_eq!(again.references[0].token, token);
        let redactor = Arc::new(reopened);
        let audit_store = Arc::new(hacienda_core::audit::InMemoryAuditStore::new(
            LOCAL_AUDIT_CONFIG_HASH,
        ));
        let viewer =
            LocalViewerState::new(Some(redactor.clone())).with_audit_store(audit_store.clone());
        let label = viewer
            .open_session("main", open_viewer_request(again))
            .expect("main window opens viewer");
        assert!(matches!(
            futures::executor::block_on(viewer.reveal_for_viewer("main", &token)),
            Err(ViewerCommandError::Unauthorized)
        ));
        let revealed = futures::executor::block_on(viewer.reveal_for_viewer(&label, &token))
            .expect("viewer reveals");
        assert_eq!(&*revealed, "alice@example.com");
        assert!(matches!(
            futures::executor::block_on(viewer.reveal_for_viewer(&label, &token)),
            Err(ViewerCommandError::Replayed)
        ));
        let audit = futures::executor::block_on(audit_store.entries(&TenantId::new("local-full")))
            .expect("persistent reveal audit");
        assert_eq!(audit.len(), 1);
        assert_eq!(audit[0].action, RedactionAction::Reveal);
        assert_eq!(audit[0].category, "EMAIL");
        assert_eq!(
            audit[0].span_hash,
            blake3::hash(b"alice@example.com").to_hex().to_string()
        );
    }

    #[test]
    fn viewer_session_rejects_main_expiry_replay_and_foreign_tokens() {
        let redactor = Arc::new(
            HaciendaLocalRedactor::from_key_store(Arc::new(TestKeyStore::default()))
                .expect("vault opens"),
        );
        let viewer = audited_viewer(Some(redactor.clone()));
        let first = redactor
            .redact_text("alice@example.com")
            .expect("tokenizes");
        let token = first.references[0].token.clone();
        assert_eq!(
            viewer.open_session("other-window", open_viewer_request(first.clone())),
            Err(ViewerCommandError::Unauthorized)
        );

        let expired_label = viewer
            .open_expired_session_for_test(open_viewer_request(first.clone()))
            .expect("expired test session");
        assert!(matches!(
            futures::executor::block_on(viewer.reveal_for_viewer(&expired_label, &token)),
            Err(ViewerCommandError::Expired)
        ));

        let label = viewer
            .open_session("main", open_viewer_request(first))
            .expect("main opens viewer");
        let foreign = redactor
            .redact_text("bob@example.com")
            .expect("tokenizes foreign value")
            .references[0]
            .token
            .clone();
        assert!(matches!(
            futures::executor::block_on(viewer.reveal_for_viewer(&label, &foreign)),
            Err(ViewerCommandError::TokenNotInSession)
        ));
        assert!(futures::executor::block_on(viewer.reveal_for_viewer(&label, &token)).is_ok());
        assert!(matches!(
            futures::executor::block_on(viewer.reveal_for_viewer(&label, &token)),
            Err(ViewerCommandError::Replayed)
        ));
    }

    #[test]
    fn manual_custom_pii_returns_only_a_reversible_token() {
        let redactor = Arc::new(
            HaciendaLocalRedactor::from_key_store(Arc::new(TestKeyStore::default()))
                .expect("vault opens"),
        );
        let viewer = audited_viewer(Some(redactor));
        let tokenized =
            futures::executor::block_on(viewer.pseudonymize_selection(ManualPseudonymRequest {
                category: "custom:project_code".into(),
                value: "Project-Z9".into(),
            }))
            .expect("manual pseudonymization");
        let serialized = serde_json::to_string(&tokenized).expect("serialize tokenized output");
        assert!(tokenized.text.starts_with("[PROJECT_CODE:local:"));
        assert!(!serialized.contains("Project-Z9"));
        let audit_entries = futures::executor::block_on(
            viewer
                .audit_store
                .as_ref()
                .expect("audit store")
                .entries(&viewer.audit_tenant),
        )
        .expect("manual action is audited");
        assert_eq!(audit_entries.len(), 1);
        assert_eq!(audit_entries[0].source, EntitySource::Manual);
        assert_eq!(audit_entries[0].action, RedactionAction::Pseudonymize);
        let serialized_audit = serde_json::to_string(&audit_entries).expect("serialize audit");
        assert!(!serialized_audit.contains("Project-Z9"));
        assert!(serialized_audit.contains(&blake3::hash(b"Project-Z9").to_hex().to_string()));

        let label = viewer
            .open_session("main", open_viewer_request(tokenized.clone()))
            .expect("viewer session");
        assert_eq!(
            &*futures::executor::block_on(
                viewer.reveal_for_viewer(&label, &tokenized.references[0].token),
            )
            .expect("authorized reveal"),
            "project-z9"
        );
    }

    #[test]
    fn missing_or_locked_vault_fails_closed() {
        struct LockedKeyStore;
        impl LocalPseudonymKeyStore for LockedKeyStore {
            fn load_or_create(
                &self,
            ) -> Result<Zeroizing<[u8; hacienda_core::redaction::KEY_BYTES]>, VaultError>
            {
                Err(VaultError::Locked)
            }
        }
        assert!(HaciendaLocalRedactor::from_key_store(Arc::new(LockedKeyStore)).is_err());
    }

    fn test_model_dir(name: &str) -> PathBuf {
        let path = env::temp_dir().join(format!(
            "hacienda-secrecy-client-{name}-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&path).expect("test model directory");
        path
    }

    #[cfg(feature = "ner-candle")]
    #[test]
    #[ignore = "loads the pinned GLiNER2 checkpoint; set HACIENDA_NER_MODEL_DIR"]
    fn pinned_gliner2_model_redacts_claude_codex_requests_and_responses() {
        let model_dir = env::var_os(NER_MODEL_DIR_ENV)
            .map(PathBuf::from)
            .expect("HACIENDA_NER_MODEL_DIR must point to the installed checkpoint");
        let redactor = HaciendaLocalRedactor::from_key_store_with_runtime(
            Arc::new(TestKeyStore::default()),
            &model_dir,
            validate_ner_model_assets,
            |config, pseudonymiser| PiiPipeline::with_pseudonymiser(config, Some(pseudonymiser)),
        )
        .expect("load and validate the pinned Candle GLiNER2 model");
        let redactor = Arc::new(redactor);
        let viewer = LocalViewerState::new(Some(Arc::clone(&redactor)));
        let model_ready_without_gateway = viewer.pii_status(false);
        assert!(!model_ready_without_gateway.provider_send_enabled);
        assert_eq!(model_ready_without_gateway.detector, "local_model");
        let configured_provider_status = viewer.pii_status(true);
        assert!(configured_provider_status.provider_send_enabled);
        assert_eq!(configured_provider_status.detector, "local_model");

        let prompt = "Maître Élodie Martin travaille à Paris pour le dossier.";
        let context = vec![hacienda_client_core::PromptContext::new(
            "Dossier Élodie Martin",
            "Élodie Martin vit à Paris ; contact elodie.martin@example.fr",
        )];
        let requests = [
            ProviderRequest::ClaudeDesktop(
                ClaudeDesktopRequest::new(
                    hacienda_client_core::ConversationId::new("model-smoke-claude"),
                    hacienda_client_core::ContextId::new("model-smoke-context-claude"),
                    prompt,
                )
                .with_context(context.clone()),
            ),
            ProviderRequest::Codex(
                CodexRequest::new(
                    hacienda_client_core::ConversationId::new("model-smoke-codex"),
                    hacienda_client_core::ContextId::new("model-smoke-context-codex"),
                    prompt,
                )
                .with_context(context),
            ),
        ];

        for request in requests {
            let outbound = Arc::new(Mutex::new(None));
            let state = ProviderCommandState::new(
                RecordingProviderTransport {
                    outbound: Arc::clone(&outbound),
                    response_text: "Élodie Martin replied from elodie.martin@example.fr in Paris."
                        .into(),
                },
                Some(SharedHaciendaRedactor(Arc::clone(&redactor))),
            )
            .with_complete_pii_detector();
            assert!(
                viewer
                    .pii_status(state.pii_detection_ready)
                    .provider_send_enabled
            );
            let response = state
                .stream(&request)
                .expect("run the model-backed request/response provider path");
            let outbound = outbound
                .lock()
                .expect("recorded outbound request lock")
                .clone()
                .expect("provider transport received a request");
            assert_eq!(outbound.provider, request.provider());

            let provider_payload =
                serde_json::to_string(&outbound).expect("serialize redacted provider DTO");
            assert!(
                provider_payload.contains("[PERSON:local:"),
                "{provider_payload}"
            );
            assert!(
                // The pinned model currently classifies this French city as ADDRESS.
                provider_payload.contains("[ADDRESS:local:"),
                "{provider_payload}"
            );
            assert!(
                provider_payload.contains("[EMAIL:local:"),
                "{provider_payload}"
            );
            for plaintext in ["Élodie Martin", "Paris", "elodie.martin@example.fr"] {
                assert!(!provider_payload.contains(plaintext), "{provider_payload}");
            }
            let person_token = outbound
                .prompt
                .references
                .iter()
                .find(|reference| reference.class == "PERSON")
                .expect("model emits a person token in the prompt")
                .token
                .clone();

            let chunks = match response {
                ProviderResponse::ClaudeDesktop(response) => response.chunks,
                ProviderResponse::Codex(response) => response.chunks,
            };
            let reply = chunks
                .iter()
                .map(|chunk| chunk.content.text.as_str())
                .collect::<Vec<_>>()
                .join("");
            assert!(reply.contains("[PERSON:local:"), "{reply}");
            assert!(reply.contains(&person_token), "{reply}");
            assert!(reply.contains("[EMAIL:local:"), "{reply}");
            assert!(reply.contains("[ADDRESS:local:"), "{reply}");
            assert!(!reply.contains("Élodie Martin"), "{reply}");
            assert!(!reply.contains("elodie.martin@example.fr"), "{reply}");
            assert!(!reply.contains("Paris"), "{reply}");
        }
    }

    #[test]
    fn missing_model_fails_closed_before_detector_initialization() {
        let missing = env::temp_dir().join(format!(
            "hacienda-secrecy-client-missing-{}",
            uuid::Uuid::new_v4()
        ));
        let result = HaciendaLocalRedactor::from_key_store_with_runtime(
            Arc::new(TestKeyStore::default()),
            &missing,
            validate_ner_model_assets,
            |_config, _pseudonymiser| panic!("detector loader must not run"),
        );
        assert!(result.is_err());
    }

    #[test]
    fn incompatible_boundary_model_fails_closed() {
        let model_dir = test_model_dir("boundary");
        std::fs::write(
            model_dir.join("config.json"),
            br#"{"model_type":"gliner2.5","counting_layer":"boundary"}"#,
        )
        .expect("incompatible config");
        let result = HaciendaLocalRedactor::from_key_store_with_runtime(
            Arc::new(TestKeyStore::default()),
            &model_dir,
            validate_ner_model_architecture,
            |_config, _pseudonymiser| panic!("detector loader must not run"),
        );
        let _ = std::fs::remove_dir_all(&model_dir);
        assert!(result.is_err());
    }

    #[test]
    fn local_model_status_requires_a_configured_provider_to_enable_send() {
        let model_dir = test_model_dir("ready");
        let redactor = HaciendaLocalRedactor::from_key_store_with_runtime(
            Arc::new(TestKeyStore::default()),
            &model_dir,
            |_| Ok(()),
            |config, pseudonymiser| {
                PiiPipeline::with_detector_and_pseudonymiser(
                    config,
                    Some(hacienda_core::pii::NerDetector::new(Arc::new(
                        InjectedNerBackend,
                    ))),
                    Some(pseudonymiser),
                )
            },
        )
        .expect("injected detector loader");
        let viewer = LocalViewerState::new(Some(Arc::new(redactor)));
        let _ = std::fs::remove_dir_all(&model_dir);
        let model_only = viewer.pii_status(false);
        assert!(!model_only.provider_send_enabled);
        assert_eq!(model_only.detector, "local_model");

        let configured_gateway = viewer.pii_status(true);
        assert!(configured_gateway.provider_send_enabled);
        assert_eq!(configured_gateway.detector, "local_model");
    }

    #[test]
    fn provider_request_contains_tokens_but_not_plaintext() {
        let redactor = HaciendaLocalRedactor::from_key_store(Arc::new(TestKeyStore::default()))
            .expect("vault opens");
        let safe = redactor
            .redact(&ProviderRequest::ClaudeDesktop(ClaudeDesktopRequest {
                conversation_id: hacienda_client_core::ConversationId::new("conversation-1"),
                context_id: hacienda_client_core::ContextId::new("context-1"),
                prompt: "Email alice@example.com".into(),
                context: vec![hacienda_client_core::PromptContext::new(
                    "Owner alice@example.com",
                    "Contact alice@example.com",
                )],
            }))
            .expect("redacts provider request");
        let payload = serde_json::to_string(&safe).expect("serializes provider payload");
        assert!(payload.contains("[EMAIL:local:"), "{payload}");
        assert!(!payload.contains("alice@example.com"), "{payload}");
    }

    #[test]
    fn regex_only_detector_cannot_authorize_provider_send() {
        let redactor = Arc::new(
            HaciendaLocalRedactor::from_key_store(Arc::new(TestKeyStore::default()))
                .expect("vault opens"),
        );
        let status = LocalViewerState::new(Some(redactor.clone())).pii_status(true);
        assert!(!status.provider_send_enabled);
        assert_eq!(status.detector, "regex_only");
        let state =
            ProviderCommandState::new(FailClosedTransport, Some(SharedHaciendaRedactor(redactor)));
        let request = ProviderRequest::ClaudeDesktop(ClaudeDesktopRequest::new(
            hacienda_client_core::ConversationId::new("conversation-1"),
            hacienda_client_core::ContextId::new("context-1"),
            "safe-looking text",
        ));
        assert!(matches!(
            state.stream(&request),
            Err(ClientError::RedactionUnavailable)
        ));
    }

    struct RecordingOpener(Mutex<Option<PathBuf>>);

    impl DesktopOpener for RecordingOpener {
        fn open(&self, path: &Path) -> Result<(), ()> {
            *self.0.lock().expect("opener lock") = Some(path.to_path_buf());
            Ok(())
        }
    }

    struct FailingOpener;

    impl DesktopOpener for FailingOpener {
        fn open(&self, _path: &Path) -> Result<(), ()> {
            Err(())
        }
    }

    fn request() -> OpenOriginalRequest {
        OpenOriginalRequest {
            document_ref: "doc-opaque".into(),
            version_ref: "version-opaque".into(),
            original_ref: "original-opaque".into(),
            viewer_ref: "viewer-opaque".into(),
            capability_ref: "capability-opaque".into(),
        }
    }

    #[test]
    fn fail_closed_runtime_never_opens_or_returns_a_path() {
        let state = OriginalViewerCommandState::fail_closed();
        let error = state.adapter.open_original(&request()).unwrap_err();
        assert_eq!(error, OriginalViewerError::Unavailable);
        assert!(!error.to_string().contains('/'));
    }

    #[test]
    fn native_viewer_requires_exact_document_version_and_capability_binding() {
        let opener = Arc::new(RecordingOpener(Mutex::new(None)));
        let viewer = NativeOriginalViewer::new(opener.clone());
        let path = std::env::temp_dir().join("hacienda-original-viewer-test.txt");
        std::fs::write(&path, "synthetic PII must stay outside renderer").expect("fixture");
        viewer.register(OriginalBinding::from(&request()), path.clone());

        let mut wrong_version = request();
        wrong_version.version_ref = "version-other".into();
        assert_eq!(
            viewer.open_original(&wrong_version),
            Err(OriginalViewerError::Unauthorized)
        );

        viewer.open_original(&request()).expect("authorized open");
        assert_eq!(opener.0.lock().expect("opener lock").as_ref(), Some(&path));
        let response = serde_json::to_string(&OpenOriginalResponse { opened: true })
            .expect("response serializes");
        assert_eq!(response, "{\"opened\":true}");
        assert!(!response.contains(path.to_string_lossy().as_ref()));
        std::fs::remove_file(path).expect("cleanup");
    }

    #[test]
    fn native_viewer_rejects_empty_refs_before_resolution() {
        let viewer = NativeOriginalViewer::new(Arc::new(RecordingOpener(Mutex::new(None))));
        let mut request = request();
        request.capability_ref.clear();
        assert_eq!(
            viewer.open_original(&request),
            Err(OriginalViewerError::InvalidRequest)
        );
    }

    #[test]
    fn failed_original_launch_still_consumes_one_shot_binding() {
        let viewer = NativeOriginalViewer::new(Arc::new(FailingOpener));
        let path =
            std::env::temp_dir().join(format!("hacienda-viewer-{}.txt", uuid::Uuid::new_v4()));
        std::fs::write(&path, b"synthetic fixture").expect("fixture");
        viewer.register(OriginalBinding::from(&request()), path.clone());

        viewer.authorize_original(&request()).expect("authorized");
        assert_eq!(
            viewer.open_original(&request()),
            Err(OriginalViewerError::OpenFailed)
        );
        assert_eq!(
            viewer.open_original(&request()),
            Err(OriginalViewerError::Unauthorized)
        );
        std::fs::remove_file(path).expect("cleanup");
    }

    #[test]
    fn selected_document_rejects_oversize_before_reading() {
        tauri::async_runtime::block_on(async {
            let path =
                std::env::temp_dir().join(format!("hacienda-size-{}.bin", uuid::Uuid::new_v4()));
            let file = std::fs::File::create(&path).expect("fixture");
            file.set_len(xberg::SecurityLimits::default().max_archive_size as u64 + 1)
                .expect("sparse file");
            drop(file);
            assert_eq!(
                read_selected_document(&path).await,
                Err("Selected document exceeds the size limit".to_owned())
            );
            std::fs::remove_file(path).expect("cleanup");
        });
    }
}
