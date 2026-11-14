#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use futures::executor::block_on;
use hacienda_core::{
    pii::{PiiPipeline, PipelineConfig},
    redaction::{KeyId, KeyResolver, PseudonymKey, Pseudonymiser, RedactionMode, KEY_BYTES},
    tenancy::TenantId,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    env,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
#[cfg(not(test))]
use tauri::Manager;
use tauri_plugin_opener::OpenerExt;
use zeroize::Zeroizing;

use hacienda_client_core::{
    ClaudeDesktopRequest, ClaudeDesktopResponse, ClientError, CodexRequest, CodexResponse,
    GatewayProviderTransport, OpaqueTokenReference, ProviderKind, ProviderRequest,
    ProviderResponse, ProviderStreamChunk, ProviderTransport, RedactedContext,
    RedactedProviderRequest, RedactionAdapter, RedactionError, TokenizedText, TransportError,
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

const LOCAL_KEYRING_SERVICE: &str = "org.hacienda.secrecy-client";
const LOCAL_KEYRING_USER: &str = "local-full-pseudonym-key-v1";

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

/// Rust-only, short-lived, one-shot viewer capability. It is intentionally neither
/// serializable nor part of provider/MCP commands. The current Tauri config has only a
/// default renderer window and no isolated viewer origin, so production has no issuer
/// and no reveal IPC command until that boundary exists.
pub struct ViewerCapability {
    expires_at: Instant,
    consumed: bool,
}

impl ViewerCapability {
    #[cfg(test)]
    fn issued_for_test() -> Self {
        Self::issue()
    }

    #[cfg(test)]
    fn issue() -> Self {
        Self {
            expires_at: Instant::now() + Duration::from_secs(30),
            consumed: false,
        }
    }

    fn consume(&mut self) -> Result<(), RedactionError> {
        if self.consumed || Instant::now() >= self.expires_at {
            return Err(RedactionError::Rejected);
        }
        self.consumed = true;
        Ok(())
    }
}

#[derive(Clone)]
struct ViewerRevealAudit {
    token_digest: [u8; 32],
    value_digest: [u8; 32],
}

/// LocalFull uses Hacienda's authenticated pseudonym tokens. The OS credential store
/// supplies the stable key; failure to open it disables this adapter.
struct HaciendaLocalRedactor {
    pipeline: PiiPipeline,
    pseudonymiser: Arc<Pseudonymiser>,
    reveal_audit: Mutex<Vec<ViewerRevealAudit>>,
}

impl HaciendaLocalRedactor {
    fn new() -> Result<Self, RedactionError> {
        Self::from_key_store(Arc::new(OsCredentialKeyStore))
    }

    fn from_key_store(store: Arc<dyn LocalPseudonymKeyStore>) -> Result<Self, RedactionError> {
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
        let pipeline = PiiPipeline::with_pseudonymiser(config, Some(pseudonymiser.clone()))
            .map_err(|_| RedactionError::Rejected)?;
        Ok(Self {
            pipeline,
            pseudonymiser,
            reveal_audit: Mutex::new(Vec::new()),
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
        capability: Option<&mut ViewerCapability>,
    ) -> Result<Zeroizing<String>, RedactionError> {
        capability.ok_or(RedactionError::Rejected)?.consume()?;
        let revealed = self
            .pseudonymiser
            .reveal(token)
            .map(Zeroizing::new)
            .map_err(|_| RedactionError::Rejected)?;
        let audit = ViewerRevealAudit {
            token_digest: *blake3::hash(token.as_bytes()).as_bytes(),
            value_digest: *blake3::hash(revealed.as_bytes()).as_bytes(),
        };
        self.reveal_audit
            .lock()
            .map_err(|_| RedactionError::Rejected)?
            .push(audit);
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

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct OriginalBinding {
    document_ref: String,
    version_ref: String,
    original_ref: String,
    viewer_ref: String,
    capability_ref: String,
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
            Self::Unavailable => "original viewer is unavailable",
            Self::InvalidRequest => "original viewer request is invalid",
            Self::Unauthorized => "original viewer capability is unavailable",
            Self::OpenFailed => "original viewer could not open the document",
        })
    }
}

trait DesktopOpener: Send + Sync {
    fn open(&self, path: &Path) -> Result<(), ()>;
}

struct TauriDesktopOpener {
    _app: tauri::AppHandle,
}

impl TauriDesktopOpener {
    fn new(app: tauri::AppHandle) -> Self {
        Self { _app: app }
    }
}

impl DesktopOpener for TauriDesktopOpener {
    fn open(&self, path: &Path) -> Result<(), ()> {
        self._app
            .opener()
            .open_path(path.to_string_lossy().to_string(), None::<String>)
            .map_err(|_| ())
    }
}

struct NativeOriginalViewer {
    bindings: Mutex<HashMap<OriginalBinding, PathBuf>>,
    opener: Arc<dyn DesktopOpener>,
}

impl NativeOriginalViewer {
    fn new(opener: Arc<dyn DesktopOpener>) -> Self {
        Self {
            bindings: Mutex::new(HashMap::new()),
            opener,
        }
    }

    #[cfg(test)]
    fn register(&self, binding: OriginalBinding, path: PathBuf) {
        self.bindings
            .lock()
            .expect("binding lock")
            .insert(binding, path);
    }
}

trait OriginalViewerAdapter: Send + Sync {
    fn open_original(&self, request: &OpenOriginalRequest) -> Result<(), OriginalViewerError>;
}

impl OriginalViewerAdapter for NativeOriginalViewer {
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
        let path = self
            .bindings
            .lock()
            .map_err(|_| OriginalViewerError::Unavailable)?
            .get(&OriginalBinding::from(request))
            .cloned()
            .ok_or(OriginalViewerError::Unauthorized)?;
        if !path.is_file() {
            return Err(OriginalViewerError::Unavailable);
        }
        self.opener
            .open(&path)
            .map_err(|_| OriginalViewerError::OpenFailed)
    }
}

struct FailClosedOriginalViewer;

impl OriginalViewerAdapter for FailClosedOriginalViewer {
    fn open_original(&self, _request: &OpenOriginalRequest) -> Result<(), OriginalViewerError> {
        Err(OriginalViewerError::Unavailable)
    }
}

struct OriginalViewerCommandState {
    adapter: Box<dyn OriginalViewerAdapter>,
}

impl OriginalViewerCommandState {
    fn fail_closed() -> Self {
        Self {
            adapter: Box::new(FailClosedOriginalViewer),
        }
    }

    fn native(app: tauri::AppHandle) -> Self {
        Self {
            adapter: Box::new(NativeOriginalViewer::new(Arc::new(
                TauriDesktopOpener::new(app),
            ))),
        }
    }
}

#[tauri::command(rename = "open-original")]
fn open_original(
    state: tauri::State<'_, OriginalViewerCommandState>,
    request: OpenOriginalRequest,
) -> Result<OpenOriginalResponse, String> {
    state
        .adapter
        .open_original(&request)
        .map_err(|error| error.to_string())?;
    Ok(OpenOriginalResponse { opened: true })
}

/// Tauri state for the transient provider path. Both adapters are injected so
/// production can use a loopback gateway while tests can use a memory seam.
pub struct ProviderCommandState {
    transport: Mutex<Box<dyn ProviderTransport + Send>>,
    redactor: Mutex<Option<Box<dyn RedactionAdapter + Send + Sync>>>,
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
            transport: Mutex::new(Box::new(transport)),
            redactor: Mutex::new(
                redactor
                    .map(|redactor| Box::new(redactor) as Box<dyn RedactionAdapter + Send + Sync>),
            ),
        }
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
        let endpoint = env::var("HACIENDA_GATEWAY_URL")
            .or_else(|_| env::var("HACIENDA_PROVIDER_ENDPOINT"))
            .map_err(|_| "Hacienda gateway is not configured".to_owned())?;
        let model = env::var("HACIENDA_PROVIDER_MODEL").unwrap_or_else(|_| "hacienda-local".into());
        let max_tokens = env::var("HACIENDA_PROVIDER_MAX_TOKENS")
            .unwrap_or_else(|_| "2048".into())
            .parse::<u32>()
            .map_err(|_| "Hacienda provider token limit is invalid".to_owned())?;
        let credential = env::var("HACIENDA_GATEWAY_TOKEN")
            .or_else(|_| env::var("HACIENDA_PROVIDER_CREDENTIAL"))
            .map_err(|_| "Hacienda gateway credential is not configured".to_owned())?;
        let transport =
            GatewayProviderTransport::new(hacienda_client_core::ProviderTransportConfig::new(
                endpoint, model, max_tokens, credential,
            ))
            .map_err(|_| "Hacienda gateway transport is invalid".to_owned())?;
        let redactor = HaciendaLocalRedactor::new()
            .map_err(|_| "Hacienda local redactor is unavailable".to_owned())?;
        Ok(Self::new(transport, Some(redactor)))
    }

    /// Install the configured runtime when possible, otherwise preserve the
    /// no-egress fail-closed contract.
    pub fn runtime() -> Self {
        Self::from_environment().unwrap_or_else(|_| Self::fail_closed())
    }

    /// Redact first, then stream through the injected token-only transport.
    pub fn stream(&self, request: &ProviderRequest) -> Result<ProviderResponse, ClientError> {
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
fn stream_claude(
    state: tauri::State<'_, ProviderCommandState>,
    request: ClaudeDesktopRequest,
) -> Result<ClaudeDesktopResponse, ClientError> {
    match state.stream(&ProviderRequest::ClaudeDesktop(request))? {
        ProviderResponse::ClaudeDesktop(response) => Ok(response),
        ProviderResponse::Codex(_) => Err(ClientError::Transport(TransportError::Failed)),
    }
}

#[tauri::command(rename = "stream-codex")]
fn stream_codex(
    state: tauri::State<'_, ProviderCommandState>,
    request: CodexRequest,
) -> Result<CodexResponse, ClientError> {
    match state.stream(&ProviderRequest::Codex(request))? {
        ProviderResponse::Codex(response) => Ok(response),
        ProviderResponse::ClaudeDesktop(_) => Err(ClientError::Transport(TransportError::Failed)),
    }
}

#[cfg(not(test))]
fn main() {
    tauri::Builder::default()
        .manage(ProviderCommandState::runtime())
        .setup(|app| {
            app.manage(OriginalViewerCommandState::native(app.handle().clone()));
            Ok(())
        })
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![
            open_original,
            stream_claude,
            stream_codex
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
        let mut capability = ViewerCapability::issued_for_test();
        let revealed = reopened
            .reveal(&token, Some(&mut capability))
            .expect("viewer reveals");
        assert_eq!(&*revealed, "alice@example.com");
        assert!(reopened.reveal(&token, Some(&mut capability)).is_err());
        let audit = reopened.reveal_audit.lock().expect("audit lock");
        assert_eq!(audit.len(), 1);
        assert_eq!(
            audit[0].token_digest,
            *blake3::hash(token.as_bytes()).as_bytes()
        );
        assert_eq!(
            audit[0].value_digest,
            *blake3::hash(b"alice@example.com").as_bytes()
        );
    }

    #[test]
    fn rehydration_requires_an_explicit_viewer_capability() {
        let redactor = HaciendaLocalRedactor::from_key_store(Arc::new(TestKeyStore::default()))
            .expect("vault opens");
        let token = redactor
            .redact_text("alice@example.com")
            .unwrap()
            .references[0]
            .token
            .clone();
        assert!(redactor.reveal(&token, None).is_err());
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

    struct RecordingOpener(Mutex<Option<PathBuf>>);

    impl DesktopOpener for RecordingOpener {
        fn open(&self, path: &Path) -> Result<(), ()> {
            *self.0.lock().expect("opener lock") = Some(path.to_path_buf());
            Ok(())
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
}
