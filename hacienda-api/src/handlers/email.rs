//! Handlers for the email processing surface (`POST /v1/email/*`).
//!
//! SECURITY (design D5): an `EmailProcessor` is NEVER shared process-wide here.
//! Each request supplies its own account/backend config and privacy mode, and a
//! fresh processor is built for that request only. The OAuth token / IMAP
//! password is resolved server-side from the environment; it is never accepted
//! in the request body, so a caller cannot point the server at an arbitrary
//! mailbox with credentials it supplied. The route's `DocumentsProcess`
//! capability gate (see `routes.rs`) is the first line of defence.

use axum::{extract::State, Json};
use hacienda_email::{
    config::EmailBackend, EmailConfig, EmailDestination, EmailOperation, EmailProcessor,
    EmailRequest, EmailSource, OAuthProvider, PipelineStep, PrivacyMode,
};
use serde::{Deserialize, Serialize};
use std::time::Instant;

use crate::{error::ApiError, extract::Json as SafeJson, state::ApiState};

/// Body for `POST /v1/email/process`.
#[derive(Debug, Deserialize)]
pub struct EmailProcessRequest {
    /// Account name used to resolve `HACIENDA_EMAIL_<ACCOUNT>_*` secrets.
    pub account: String,
    /// Backend selection. `maildir` needs a `path`; `oauth` needs a provider.
    pub backend: EmailBackendDto,
    /// Privacy mode for this request (per-tenant choice, design D1).
    #[serde(default)]
    pub privacy_mode: PrivacyModeDto,
    /// The operation to perform.
    pub operation: EmailOperationDto,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum EmailBackendDto {
    Maildir { path: String },
    OAuth {
        /// Optional server URL; empty uses the provider's default IMAP endpoint.
        #[serde(default)]
        server: String,
        username: String,
        provider: OAuthProviderDto,
    },
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum OAuthProviderDto {
    Gmail,
    Outlook,
}

#[derive(Debug, Deserialize)]
#[serde(default)]
pub struct PrivacyModeDto {
    #[serde(default)]
    pub mode: PrivacyModeValue,
}

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum PrivacyModeValue {
    #[default]
    Redact,
    Pseudonymize,
}

impl Default for PrivacyModeDto {
    fn default() -> Self {
        Self { mode: PrivacyModeValue::Redact }
    }
}

#[derive(Debug, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case")]
pub enum EmailOperationDto {
    Archive { source: String, output: String, #[serde(default)] since: Option<String> },
    ExtractNER { source: String, output: String },
    Redact { source: String, output: String },
    IndexForRag { source: String, #[serde(default)] index: Option<String> },
    Pipeline { source: String, output: String },
}

/// Response body for an email operation.
#[derive(Debug, Serialize)]
pub struct EmailProcessResponse {
    pub success: bool,
    pub processed_count: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub redaction_stats: Option<hacienda_email::models::RedactionStats>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rag_stats: Option<hacienda_email::models::RagIndexStats>,
    pub errors: Vec<String>,
    pub processing_time_ms: u64,
}

fn to_api_error(e: hacienda_email::EmailError) -> ApiError {
    ApiError::invalid_request(format!("email operation failed: {e}"))
}

fn build_config(body: &EmailProcessRequest) -> Result<EmailConfig, ApiError> {
    let backend = match &body.backend {
        EmailBackendDto::Maildir { path } => EmailBackend::Maildir { path: path.clone() },
        EmailBackendDto::OAuth { server, username, provider } => {
            let provider = match provider {
                OAuthProviderDto::Gmail => OAuthProvider::Gmail,
                OAuthProviderDto::Outlook => OAuthProvider::Outlook,
            };
            EmailBackend::OAuth {
                server: server.clone(),
                username: username.clone(),
                provider,
            }
        }
    };
    let mode = match body.privacy_mode.mode {
        PrivacyModeValue::Redact => PrivacyMode::Redact,
        PrivacyModeValue::Pseudonymize => PrivacyMode::Pseudonymize,
    };
    let mut config = EmailConfig {
        backend,
        account: body.account.clone(),
        ..Default::default()
    };
    config.redaction.mode = mode;
    Ok(config)
}

fn source_of(path: String) -> EmailSource {
    EmailSource::Maildir { path }
}

fn dest_of(path: String) -> EmailDestination {
    EmailDestination::Maildir { path }
}

/// `POST /v1/email/process`
///
/// OpenAPI body schemas are intentionally omitted here: the request embeds
/// `hacienda_email` enum types which do not implement utoipa's `ToSchema`, and
/// adding that derive across the email crate would leak an API concern into the
/// domain layer. The route, auth requirement and status codes are still documented.
#[utoipa::path(
    post,
    path = "/v1/email/process",
    tag = "email",
    operation_id = "processEmail",
    security(("bearerAuth" = [])),
    responses(
        (status = 200, description = "Email operation result"),
        (status = 400, description = "Invalid email request"),
        (status = 401, description = "Missing or invalid credentials"),
        (status = 403, description = "Caller lacks documents:process")
    )
)]
pub async fn process_email(
    State(_state): State<ApiState>,
    SafeJson(body): SafeJson<EmailProcessRequest>,
) -> Result<Json<EmailProcessResponse>, ApiError> {
    let start = Instant::now();
    let config = build_config(&body)?;
    // Per-request processor: never shared across tenants (design D5).
    let processor = EmailProcessor::new(config).await.map_err(to_api_error)?;

    let operation = match &body.operation {
        EmailOperationDto::Archive { source, output, since } => EmailOperation::Archive {
            source: source_of(source.clone()),
            destination: dest_of(output.clone()),
            since: since.clone(),
            before: None,
            mailbox: Some("INBOX".to_string()),
            include_attachments: true,
        },
        EmailOperationDto::ExtractNER { source, output } => EmailOperation::ExtractNER {
            source: source_of(source.clone()),
            output: output.clone(),
            model: None,
        },
        EmailOperationDto::Redact { source, output } => EmailOperation::Redact {
            source: source_of(source.clone()),
            destination: dest_of(output.clone()),
            rules: None,
        },
        EmailOperationDto::IndexForRag { source, index } => EmailOperation::IndexForRag {
            source: source_of(source.clone()),
            index_name: index.clone(),
            chunk_size: None,
        },
        EmailOperationDto::Pipeline { source, output } => EmailOperation::Pipeline {
            source: source_of(source.clone()),
            destination: dest_of(output.clone()),
            steps: vec![
                PipelineStep::Archive,
                PipelineStep::ExtractAttachments,
                PipelineStep::Ner,
                PipelineStep::RagIndex,
                PipelineStep::Redact,
            ],
        },
    };

    let response = processor
        .process(EmailRequest { operation, config_overrides: None })
        .await
        .map_err(to_api_error)?;

    Ok(Json(EmailProcessResponse {
        success: response.success,
        processed_count: response.processed_count,
        redaction_stats: response.redaction_stats,
        rag_stats: response.rag_stats,
        errors: response.errors,
        processing_time_ms: start.elapsed().as_millis() as u64,
    }))
}
