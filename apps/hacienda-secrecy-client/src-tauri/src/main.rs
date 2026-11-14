#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use futures::executor::block_on;
use hacienda_core::pii::{PiiPipeline, PipelineConfig};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    env,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};
#[cfg(not(test))]
use tauri::Manager;
use tauri_plugin_opener::OpenerExt;

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

/// Hacienda's local deterministic PII pipeline is the only redaction capability
/// injected into the provider command state. It deliberately starts regex-only;
/// enabling a model is a deployment decision and a failed model load must keep the
/// client unavailable rather than silently downgrade.
struct HaciendaLocalRedactor {
    pipeline: PiiPipeline,
}

impl HaciendaLocalRedactor {
    fn new() -> Result<Self, RedactionError> {
        PiiPipeline::new(PipelineConfig::default())
            .map(|pipeline| Self { pipeline })
            .map_err(|_| RedactionError::Rejected)
    }

    fn tokenize(&self, text: &str) -> Result<TokenizedText, RedactionError> {
        let result = block_on(self.pipeline.process(text)).map_err(|_| RedactionError::Rejected)?;
        let mut references = Vec::new();
        for entity in result.entities {
            let reference =
                OpaqueTokenReference::new(entity.redact_template, entity.category.to_string());
            if !references.contains(&reference) {
                references.push(reference);
            }
        }
        Ok(TokenizedText::new(result.redacted_text, references))
    }
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
                        label: item.label.clone(),
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
