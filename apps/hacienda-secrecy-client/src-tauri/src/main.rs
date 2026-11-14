#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use futures::executor::block_on;
use hacienda_core::{
    audit::{AuditEntryInput, AuditStore, EntitySource, FileAuditStore, NodeId, RedactionAction},
    pii::{PiiCategory, PiiPipeline, PipelineConfig},
    redaction::{KeyId, KeyResolver, PseudonymKey, Pseudonymiser, RedactionMode, KEY_BYTES},
    tenancy::TenantId,
};
use serde::{Deserialize, Serialize};
#[cfg(test)]
use std::path::{Path, PathBuf};
use std::{
    collections::{HashMap, HashSet, VecDeque},
    env,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
#[cfg(not(test))]
use tauri::Manager;
use tauri::{WebviewUrl, WebviewWindow, WebviewWindowBuilder};
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

const MAIN_WINDOW_LABEL: &str = "main";
const LOCAL_VIEWER_PREFIX: &str = "local-viewer-";
const VIEWER_SESSION_TTL: Duration = Duration::from_secs(60);
const MAX_VIEWER_AUDIT_ENTRIES: usize = 256;

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
    reveal_audit: Mutex<VecDeque<ViewerRevealAudit>>,
    pii_detection_complete: bool,
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
        // This Tauri package does not build the `ner-candle` backend or provision
        // verified GLiNER weights, so regex-only detection can never authorize egress.
        let pii_detection_complete = false;
        let pipeline = PiiPipeline::with_pseudonymiser(config, Some(pseudonymiser.clone()))
            .map_err(|_| RedactionError::Rejected)?;
        Ok(Self {
            pipeline,
            pseudonymiser,
            reveal_audit: Mutex::new(VecDeque::new()),
            pii_detection_complete,
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
        let audit = ViewerRevealAudit {
            token_digest: *blake3::hash(token.as_bytes()).as_bytes(),
            value_digest: *blake3::hash(revealed.as_bytes()).as_bytes(),
        };
        let mut reveal_audit = self
            .reveal_audit
            .lock()
            .map_err(|_| RedactionError::Rejected)?;
        if reveal_audit.len() == MAX_VIEWER_AUDIT_ENTRIES {
            reveal_audit.pop_front();
        }
        reveal_audit.push_back(audit);
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

    fn fail_closed() -> Self {
        Self::new(None)
    }

    fn pii_status(&self) -> LocalPiiStatusResponse {
        let complete = self
            .redactor
            .as_ref()
            .is_some_and(|redactor| redactor.pii_detection_complete);
        LocalPiiStatusResponse {
            provider_send_enabled: complete,
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

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
#[cfg(test)]
struct OriginalBinding {
    document_ref: String,
    version_ref: String,
    original_ref: String,
    viewer_ref: String,
    capability_ref: String,
}

#[cfg(test)]
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

#[cfg(test)]
trait DesktopOpener: Send + Sync {
    fn open(&self, path: &Path) -> Result<(), ()>;
}

#[cfg(test)]
struct NativeOriginalViewer {
    bindings: Mutex<HashMap<OriginalBinding, PathBuf>>,
    opener: Arc<dyn DesktopOpener>,
}

#[cfg(test)]
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

#[cfg(test)]
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

    /// Production stays fail-closed until a native file picker and trusted
    /// ingestion command can register the selected path with an opaque binding.
    fn unavailable() -> Self {
        Self::fail_closed()
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
            transport: Mutex::new(Box::new(transport)),
            redactor: Mutex::new(
                redactor
                    .map(|redactor| Box::new(redactor) as Box<dyn RedactionAdapter + Send + Sync>),
            ),
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

    fn runtime_with_viewer() -> (Self, LocalViewerState) {
        let redactor = HaciendaLocalRedactor::new().ok().map(Arc::new);
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
fn local_pii_status(state: tauri::State<'_, LocalViewerState>) -> LocalPiiStatusResponse {
    state.pii_status()
}

#[cfg(not(test))]
fn main() {
    let (provider_state, viewer_state) = ProviderCommandState::runtime_with_viewer();
    tauri::Builder::default()
        .manage(provider_state)
        .setup(|app| {
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
            let viewer_state = match audit_store {
                Some(store) => viewer_state.with_audit_store(store),
                None => viewer_state,
            };
            app.manage(viewer_state);
            app.manage(OriginalViewerCommandState::unavailable());
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
            local_pii_status
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
        let viewer = audited_viewer(Some(redactor.clone()));
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
        let audit = redactor.reveal_audit.lock().expect("audit lock");
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
        let status = LocalViewerState::new(Some(redactor.clone())).pii_status();
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
        let state = OriginalViewerCommandState::unavailable();
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
