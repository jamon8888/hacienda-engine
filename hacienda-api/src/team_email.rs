//! TeamOnPremise email integration surface.
//!
//! This module is the *only* email surface exposed to a desktop client that does not run
//! the email engine in-process. It deliberately never accepts a filesystem path, an
//! OAuth token, a password, a broker command, or raw message text from the wire:
//!
//! * [`EmailIngestRequest`] carries a server-provisioned `source_ref` + `mailbox_ref`.
//!   The handler resolves those opaque references to a tenant-confined Maildir path and
//!   then delegates to the already-audited, already redacted
//!   [`crate::handlers::email`] handlers. The renderer therefore cannot steer the server
//!   at another tenant's mailbox or at an arbitrary absolute path.
//! * [`ViewerCapabilityRequest`] mints an opaque, short-lived capability bound to the
//!   authenticated tenant, the requesting principal, a document, an email and (optionally)
//!   an attachment. Reveal/open happen **server-side**, are audited, and the response DTOs
//!   are content-free — no plaintext, path, or raw token ever reaches a renderer DTO.
//! * [`EmailSearchRequest`] delegates to the same tenant-scoped, PII-redacted RAG retrieval
//!   path as document search and projects the result onto a token-only DTO.
//!
//! Sync and async ingestion share the exact `EmailOperationResultDto` used by the core
//! email handlers, so the two paths cannot diverge on confidentiality or shape.

use axum::{extract::Path, extract::State, http::request::Parts, http::StatusCode, Json};
use hacienda_core::auth::{AuthContext, Caller, Capability};
use hacienda_rag::{RetrieveMode, RetrieveQuery};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use utoipa::ToSchema;
use uuid::Uuid;

use crate::dto::{AsyncJobResponse, EmailOperationResultDto};
use crate::error::ApiError;
use crate::extract::Json as SafeJson;
use crate::handlers::{
    email::{
        process_email, process_email_async, EmailBackendDto, EmailOperationDto,
        EmailProcessRequest, PrivacyModeDto, PrivacyModeValue,
    },
    extract_auth_context,
    rag::{retrieve, scope_collection_name},
};
use crate::state::ApiState;

/// Maximum lifetime of a viewer capability. Five minutes is long enough for a user to
/// open one message, short enough that a captured reference is worthless shortly after.
pub const VIEWER_CAPABILITY_TTL: Duration = Duration::from_secs(300);

// ── Wire DTOs ──────────────────────────────────────────────────────────────────────────

/// One server-provisioned email source. `source_ref` and `mailbox_refs` are opaque
/// identifiers; no path, endpoint, credential or account name is ever serialized.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct EmailSourceDto {
    pub source_ref: String,
    pub mailbox_refs: Vec<String>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct EmailSourcesResponse {
    pub sources: Vec<EmailSourceDto>,
}

/// Body of `POST /v1/team/email/ingest` and `/ingest/async`.
///
/// `deny_unknown_fields` makes an attempt to smuggle `backend`, `path`, `token`,
/// `password`, `server`, `token_command`, or raw message text a hard 400. The server
/// derives the account from the authenticated tenant and the path from its own registry.
#[derive(Debug, Clone, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct EmailIngestRequest {
    /// Opaque, server-provisioned source identifier.
    pub source_ref: String,
    /// Opaque mailbox identifier, valid only for `source_ref`.
    pub mailbox_ref: String,
    pub operation: EmailIngestOperationDto,
    #[serde(default)]
    pub privacy_mode: EmailPrivacyDto,
    #[serde(default)]
    pub dossier_id: Option<String>,
}

/// Operations are selected by name only. Unlike the low-level email DTO there are no
/// `source`/`output` path fields: the server provisions both from the refs.
#[derive(Debug, Clone, Copy, Deserialize, ToSchema, PartialEq, Eq)]
#[serde(tag = "operation", rename_all = "snake_case")]
pub enum EmailIngestOperationDto {
    IndexForRag,
    /// Canonical wire spelling; the legacy `extract_n_e_r` spelling is intentionally
    /// not accepted on this surface.
    ExtractNer,
    Redact,
    Pipeline,
}

#[derive(Debug, Clone, Copy, Deserialize, Default, ToSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EmailPrivacyDto {
    #[default]
    Redact,
    /// Parsed only so the handlers can reject it explicitly with a 400. The
    /// TeamOnPremise surface never mints deterministic tenant-keyed tokens.
    Pseudonymize,
}

/// Body of `POST /v1/team/viewer/capability`. All fields are opaque references; the
/// `token_ref` is a pseudonym from a redacted projection, never a raw PII value.
#[derive(Debug, Clone, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ViewerCapabilityRequest {
    pub document_ref: String,
    pub email_ref: String,
    #[serde(default)]
    pub attachment_ref: Option<String>,
    pub token_ref: String,
    #[serde(default)]
    pub purpose: Option<String>,
}

/// The only thing the renderer ever receives for a viewer session. It contains no path,
/// no plaintext, and no credential — just an opaque reference and its scope.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ViewerCapabilityDto {
    pub capability_ref: String,
    pub expires_at: String,
    pub document_ref: String,
    pub email_ref: String,
    pub attachment_ref: Option<String>,
    pub delivery: ViewerDeliveryDto,
}

#[derive(Debug, Clone, Copy, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ViewerDeliveryDto {
    ServerSide,
}

#[derive(Debug, Clone, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ViewerActionRequest {
    pub capability_ref: String,
}

/// Content-free result of a server-side reveal. The plaintext is consumed inside the
/// trusted server/viewer boundary; only the outcome and counted references cross IPC.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ViewerRevealResultDto {
    pub capability_ref: String,
    pub revealed: bool,
    pub document_ref: String,
    pub email_ref: String,
}

/// One header of a content-safe viewer projection.
///
/// `name` is drawn from a fixed allowlist; `token_only` is either a pseudonym
/// token (`[CATEGORY:key:…]`) or the content-free marker `[REDACTED]`. A clear
/// subject, address or display name can never occupy this field.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ViewerHeaderDto {
    pub name: String,
    pub token_only: String,
}

/// Marks how the projection was produced. Always `redacted_token_only` — there
/// is deliberately no `clear` variant a renderer could request.
#[derive(Debug, Clone, Copy, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ViewerProjectionKindDto {
    RedactedTokenOnly,
}

/// The typed, content-safe result of consuming a viewer capability.
///
/// This replaces the old `opened: true` boolean: the server actually opens the
/// capability-bound message from the tenant's redacted RAG projection and
/// returns headers, body and attachment *references*. No clear-text message,
/// filename or path is ever serialized here — the plaintext reveal, when
/// needed, stays a separate capability-scoped, audited server-side operation.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ViewerProjectionDto {
    pub capability_ref: String,
    pub document_ref: String,
    pub email_ref: String,
    /// Token-only headers (allowlisted names only).
    pub headers: Vec<ViewerHeaderDto>,
    /// Redacted, token-only body, bounded in size.
    pub body_token_only: String,
    /// Opaque attachment references; no bytes, filenames or paths.
    pub attachment_refs: Vec<String>,
    pub projection_kind: ViewerProjectionKindDto,
}

/// Content-free audit event. Refs are opaque and there is no span text, subject, address,
/// filename or path field by construction.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ViewerAuditEventDto {
    pub sequence: u64,
    pub timestamp: String,
    pub action: String,
    pub tenant: String,
    pub principal: String,
    pub document_ref: String,
    pub email_ref: String,
    pub attachment_ref: Option<String>,
    pub capability_ref: String,
    pub outcome: String,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ViewerAuditResponse {
    pub events: Vec<ViewerAuditEventDto>,
}

#[derive(Debug, Clone, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct EmailSearchRequest {
    /// Restricts the search to a server-provisioned source (no paths on the wire).
    pub source_ref: String,
    pub mailbox_ref: String,
    pub query_text: String,
    #[serde(default)]
    pub top_k: Option<u32>,
    #[serde(default)]
    pub dossier_id: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, ToSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EmailSourceKindDto {
    Email,
}

#[derive(Debug, Clone, Copy, Serialize, ToSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ViewerPolicyDto {
    CapabilityRequired,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct EmailSearchHitDto {
    /// Opaque document reference (a hash of the store id, never the id or path).
    pub document_ref: String,
    /// Opaque external reference, hashed so no `email:<account>:...` identity leaks.
    pub external_ref: String,
    pub source_kind: EmailSourceKindDto,
    pub mime: Option<String>,
    pub dossier_id: Option<String>,
    pub version_ref: Option<String>,
    pub ordinal: u32,
    pub score: f32,
    /// Already redacted by the shared RAG path before serialization.
    pub projection: String,
    pub viewer_policy: ViewerPolicyDto,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct EmailSearchResponse {
    pub hits: Vec<EmailSearchHitDto>,
}

// ── Server-provisioned source registry ─────────────────────────────────────────────────

#[derive(Debug, Clone, Deserialize)]
struct SourcesConfig {
    #[serde(default)]
    sources: Vec<SourceConfig>,
}

#[derive(Debug, Clone, Deserialize)]
struct SourceConfig {
    source_ref: String,
    #[serde(default)]
    mailboxes: Vec<MailboxConfig>,
}

#[derive(Debug, Clone, Deserialize)]
struct MailboxConfig {
    mailbox_ref: String,
    /// Relative Maildir path under the authenticated tenant's root. Absolute paths and
    /// parent traversal are rejected at load time.
    path: String,
}

#[derive(Debug, Clone)]
struct SourceRecord {
    /// mailbox_ref → relative Maildir path.
    mailboxes: HashMap<String, String>,
}

/// Server-owned allowlist of email sources. It is constructed from operator configuration
/// at process start; a request can select a ref but can never introduce a path.
#[derive(Debug, Clone, Default)]
pub struct EmailSourceRegistry {
    sources: HashMap<String, SourceRecord>,
}

fn safe_relative_path(path: &str) -> bool {
    let path = std::path::Path::new(path);
    !path.as_os_str().is_empty()
        && !path.is_absolute()
        && !path.components().any(|c| {
            matches!(
                c,
                std::path::Component::ParentDir | std::path::Component::Prefix(_)
            )
        })
}

fn safe_ref(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 256
        && !value.contains('/')
        && !value.contains('\\')
        && !value.starts_with('.')
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | ':'))
}

/// A pseudonym token such as `[EMAIL:k1:ABC]` is an opaque reference too, but
/// its canonical wire form carries brackets. This is a distinct grammar from
/// `safe_ref`: still no path separators, whitespace, `@`, or raw-text carriers,
/// so a plaintext address cannot be smuggled in as a `token_ref`.
fn safe_token_ref(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 256
        && !value.contains('/')
        && !value.contains('\\')
        && !value.contains('@')
        && !value.chars().any(char::is_whitespace)
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | ':' | '[' | ']'))
}

impl EmailSourceRegistry {
    /// Build the registry from the `HACIENDA_EMAIL_SOURCES` JSON configuration. When no
    /// configuration is provided but a mailbox root exists, one conventional source is
    /// provisioned so a correctly configured server is usable out of the box. Otherwise
    /// the registry is empty and all source-dependent routes fail closed.
    pub fn from_env() -> Self {
        let mut registry = Self::default();
        if let Ok(raw) = std::env::var("HACIENDA_EMAIL_SOURCES") {
            if let Ok(config) = serde_json::from_str::<SourcesConfig>(&raw) {
                for source in config.sources {
                    if !safe_ref(&source.source_ref) {
                        continue;
                    }
                    let mut mailboxes = HashMap::new();
                    for mailbox in source.mailboxes {
                        if safe_ref(&mailbox.mailbox_ref) && safe_relative_path(&mailbox.path) {
                            mailboxes.insert(mailbox.mailbox_ref, mailbox.path);
                        }
                    }
                    if !mailboxes.is_empty() {
                        registry
                            .sources
                            .insert(source.source_ref, SourceRecord { mailboxes });
                    }
                }
            }
        }
        if registry.sources.is_empty() && std::env::var_os("HACIENDA_MAILBOX_ROOT").is_some() {
            let mut mailboxes = HashMap::new();
            mailboxes.insert("inbox".to_string(), "INBOX".to_string());
            registry
                .sources
                .insert("default".to_string(), SourceRecord { mailboxes });
        }
        registry
    }

    /// Construct a registry from explicit configuration (used by tests and by a host that
    /// loads sources from its own secret/config store).
    pub fn from_parts(sources: Vec<(String, Vec<(String, String)>)>) -> Self {
        let mut registry = Self::default();
        for (source_ref, mailboxes) in sources {
            if !safe_ref(&source_ref) {
                continue;
            }
            let mailboxes: HashMap<_, _> = mailboxes
                .into_iter()
                .filter(|(reference, path)| safe_ref(reference) && safe_relative_path(path))
                .collect();
            if !mailboxes.is_empty() {
                registry
                    .sources
                    .insert(source_ref, SourceRecord { mailboxes });
            }
        }
        registry
    }

    fn maildir_for(&self, source_ref: &str, mailbox_ref: &str) -> Option<String> {
        self.sources
            .get(source_ref)
            .and_then(|source| source.mailboxes.get(mailbox_ref))
            .cloned()
    }

    pub fn sources_dto(&self) -> Vec<EmailSourceDto> {
        let mut sources: Vec<_> = self
            .sources
            .iter()
            .map(|(source_ref, record)| {
                let mut mailbox_refs: Vec<_> = record.mailboxes.keys().cloned().collect();
                mailbox_refs.sort();
                EmailSourceDto {
                    source_ref: source_ref.clone(),
                    mailbox_refs,
                }
            })
            .collect();
        sources.sort_by(|a, b| a.source_ref.cmp(&b.source_ref));
        sources
    }
}

// ── Opaque, tenant-bound viewer capability ─────────────────────────────────────────────

#[derive(Debug, Clone)]
struct ViewerCapabilityRecord {
    tenant: String,
    principal: String,
    document_ref: String,
    email_ref: String,
    attachment_ref: Option<String>,
    token_ref: String,
    expires_at: Instant,
}

#[derive(Debug, Clone)]
struct ViewerAuditEvent {
    sequence: u64,
    timestamp: String,
    action: String,
    tenant: String,
    principal: String,
    document_ref: String,
    email_ref: String,
    attachment_ref: Option<String>,
    capability_ref: String,
    outcome: String,
}

/// In-memory capability and audit store. A capability dies with the process, expires
/// after [`VIEWER_CAPABILITY_TTL`], and is usable only by the tenant + principal that
/// minted it. Every decision (including denial) is recorded content-free.
#[derive(Debug)]
pub struct ViewerCapabilityStore {
    capabilities: Mutex<HashMap<String, ViewerCapabilityRecord>>,
    /// Refs already consumed by an `open`. Kept (rather than simply deleted) so
    /// a replay is distinguishable in the audit trail from an unknown ref while
    /// still returning the same generic 404 to the caller.
    consumed: Mutex<std::collections::HashSet<String>>,
    audit: Mutex<Vec<ViewerAuditEvent>>,
    ttl: Duration,
}

impl Default for ViewerCapabilityStore {
    fn default() -> Self {
        Self::new(VIEWER_CAPABILITY_TTL)
    }
}

impl ViewerCapabilityStore {
    pub fn new(ttl: Duration) -> Self {
        Self {
            capabilities: Mutex::new(HashMap::new()),
            consumed: Mutex::new(std::collections::HashSet::new()),
            audit: Mutex::new(Vec::new()),
            ttl,
        }
    }

    fn now_wire() -> String {
        chrono::Utc::now().to_rfc3339()
    }

    fn issue(
        &self,
        request: &ViewerCapabilityRequest,
        tenant: &str,
        principal: &str,
    ) -> ViewerCapabilityDto {
        // Two UUIDs (256 bits of entropy) behind a typed prefix: unguessable, carries no
        // tenant/document/path information, and not derivable from any renderer input.
        let capability_ref = format!(
            "vcap_{}{}",
            Uuid::new_v4().simple(),
            Uuid::new_v4().simple()
        );
        // `expires_at` is the actual expiry instant, not the issue time. Both clocks are
        // derived from the same TTL so the rendered deadline cannot outlive the record.
        let expires_at = Instant::now() + self.ttl;
        let expires_at_wire = (chrono::Utc::now()
            + chrono::Duration::from_std(self.ttl).unwrap_or_default())
        .to_rfc3339();
        let record = ViewerCapabilityRecord {
            tenant: tenant.to_string(),
            principal: principal.to_string(),
            document_ref: request.document_ref.clone(),
            email_ref: request.email_ref.clone(),
            attachment_ref: request.attachment_ref.clone(),
            token_ref: request.token_ref.clone(),
            expires_at,
        };
        // A poisoned mutex means a previous panic left state indeterminate; recover the
        // guard but clear the store so no capability minted before the panic can be used.
        let poisoned = self.capabilities.is_poisoned();
        let mut capabilities = self
            .capabilities
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        if poisoned {
            capabilities.clear();
            if let Ok(mut consumed) = self.consumed.lock() {
                consumed.clear();
            }
        }
        capabilities.insert(capability_ref.clone(), record);
        ViewerCapabilityDto {
            capability_ref,
            expires_at: expires_at_wire,
            document_ref: request.document_ref.clone(),
            email_ref: request.email_ref.clone(),
            attachment_ref: request.attachment_ref.clone(),
            delivery: ViewerDeliveryDto::ServerSide,
        }
    }

    /// Authorize use of a capability. Returns the record only when the reference exists,
    /// is unexpired, and belongs to exactly this tenant and principal.
    fn authorize(
        &self,
        capability_ref: &str,
        ctx: &AuthContext,
        action: &str,
    ) -> Result<ViewerCapabilityRecord, ApiError> {
        let tenant = ctx.tenant.as_str();
        let principal = ctx.principal_id.as_str();
        let record = {
            let mut capabilities = match self.capabilities.lock() {
                Ok(guard) => guard,
                Err(_) => return Err(ApiError::internal()),
            };
            let record = capabilities.get(capability_ref).cloned();
            // Opportunistic expiry sweep: an expired capability is removed on first use.
            capabilities.retain(|_, value| value.expires_at > Instant::now());
            record
        };
        let record = match record {
            Some(record) if record.expires_at > Instant::now() => record,
            _ => {
                self.audit_event(None, ctx, action, "not_found", capability_ref);
                return Err(ApiError::not_found());
            }
        };
        if record.tenant != tenant || record.principal != principal {
            // Cross-tenant (or cross-principal) use is denied and audited. The response is
            // deliberately generic: no hint whether the reference existed or who owned it.
            self.audit_event(Some(&record), ctx, action, "denied", capability_ref);
            return Err(ApiError::forbidden());
        }
        Ok(record)
    }

    /// Authorize **and consume** a capability in one atomic step. `open` returns
    /// content, so its capability is single-use: a captured reference cannot be
    /// replayed to re-open the same message. Every decision — including replay,
    /// expiry and cross-tenant denial — is audited content-free.
    fn consume(
        &self,
        capability_ref: &str,
        ctx: &AuthContext,
        action: &str,
    ) -> Result<ViewerCapabilityRecord, ApiError> {
        enum Outcome {
            Consumed(ViewerCapabilityRecord),
            Replayed,
            NotFound,
            Denied(ViewerCapabilityRecord),
        }

        let outcome = {
            let mut capabilities = match self.capabilities.lock() {
                Ok(guard) => guard,
                Err(_) => return Err(ApiError::internal()),
            };
            let mut consumed = match self.consumed.lock() {
                Ok(guard) => guard,
                Err(_) => return Err(ApiError::internal()),
            };
            if consumed.contains(capability_ref) {
                Outcome::Replayed
            } else {
                let record = capabilities.get(capability_ref).cloned();
                // Opportunistic expiry sweep: an expired capability is removed on use.
                capabilities.retain(|_, value| value.expires_at > Instant::now());
                match record {
                    Some(record) if record.expires_at > Instant::now() => {
                        if record.tenant != ctx.tenant.as_str()
                            || record.principal != ctx.principal_id.as_str()
                        {
                            Outcome::Denied(record)
                        } else {
                            capabilities.remove(capability_ref);
                            // Capability TTL is minutes; a bounded replay set is
                            // enough to audit recent replays without growing
                            // forever. Old refs still fail as "not found".
                            if consumed.len() >= 10_000 {
                                consumed.clear();
                            }
                            consumed.insert(capability_ref.to_string());
                            Outcome::Consumed(record)
                        }
                    }
                    _ => Outcome::NotFound,
                }
            }
        };

        match outcome {
            Outcome::Consumed(record) => {
                self.audit_event(
                    Some(&record),
                    ctx,
                    action,
                    "consumed_projection",
                    capability_ref,
                );
                Ok(record)
            }
            Outcome::Replayed => {
                self.audit_event(None, ctx, action, "replayed", capability_ref);
                Err(ApiError::not_found())
            }
            Outcome::NotFound => {
                self.audit_event(None, ctx, action, "not_found", capability_ref);
                Err(ApiError::not_found())
            }
            Outcome::Denied(record) => {
                self.audit_event(Some(&record), ctx, action, "denied", capability_ref);
                Err(ApiError::forbidden())
            }
        }
    }

    fn audit_event(
        &self,
        record: Option<&ViewerCapabilityRecord>,
        ctx: &AuthContext,
        action: &str,
        outcome: &str,
        capability_ref: &str,
    ) {
        let mut audit = match self.audit.lock() {
            Ok(guard) => guard,
            Err(_) => return,
        };
        let sequence = audit.len() as u64 + 1;
        audit.push(ViewerAuditEvent {
            sequence,
            timestamp: Self::now_wire(),
            action: action.to_string(),
            tenant: ctx.tenant.as_str().to_string(),
            principal: ctx.principal_id.clone(),
            document_ref: record.map(|r| r.document_ref.clone()).unwrap_or_default(),
            email_ref: record.map(|r| r.email_ref.clone()).unwrap_or_default(),
            attachment_ref: record.and_then(|r| r.attachment_ref.clone()),
            capability_ref: capability_ref.to_string(),
            outcome: outcome.to_string(),
        });
    }

    fn audit_for_tenant(&self, tenant: &str) -> ViewerAuditResponse {
        let audit = match self.audit.lock() {
            Ok(guard) => guard,
            Err(_) => return ViewerAuditResponse { events: Vec::new() },
        };
        let events = audit
            .iter()
            .filter(|event| event.tenant == tenant)
            .map(|event| ViewerAuditEventDto {
                sequence: event.sequence,
                timestamp: event.timestamp.clone(),
                action: event.action.clone(),
                tenant: event.tenant.clone(),
                principal: event.principal.clone(),
                document_ref: event.document_ref.clone(),
                email_ref: event.email_ref.clone(),
                attachment_ref: event.attachment_ref.clone(),
                capability_ref: event.capability_ref.clone(),
                outcome: event.outcome.clone(),
            })
            .collect();
        ViewerAuditResponse { events }
    }
}

// ── Helpers ─────────────────────────────────────────────────────────────────────────────

fn authenticated(parts: &Parts) -> Result<Arc<AuthContext>, ApiError> {
    extract_auth_context(parts).ok_or_else(ApiError::unauthenticated)
}

/// Hash an internal identifier into an opaque wire reference. Public so tests
/// can compute the exact reference a renderer would receive without exposing
/// the underlying document/email id.
pub fn opaque_ref(value: &str) -> String {
    let digest = blake3::hash(value.as_bytes()).to_hex();
    format!("ref_{}", &digest[..32])
}

/// Header values are short and structured: keep only bracketed pseudonym tokens
/// and replace everything else with a content-free marker. This is deliberately
/// stricter than the body projection — a clear subject or display name must
/// never reach a renderer even if a store row was populated by a caller that
/// bypassed the email redaction path.
fn safe_header_value(value: &str) -> String {
    let parts: Vec<&str> = value
        .split_whitespace()
        .map(|part| {
            if safe_token_ref(part) && part.starts_with('[') && part.ends_with(']') {
                part
            } else {
                "[REDACTED]"
            }
        })
        .collect();
    let joined = parts.join(" ");
    if joined.chars().count() > 512 {
        "[REDACTED]".to_string()
    } else {
        joined
    }
}

/// Defense-in-depth body projection. The email ingestion path already redacts
/// `full_text` at rest; this guard additionally removes obvious clear-text
/// carriers (addresses, long digit runs) in case a row was written through a
/// different path, and bounds the payload size.
fn safe_body(value: &str) -> String {
    let bounded: String = value.chars().take(65_536).collect();
    let chars: Vec<char> = bounded.chars().collect();
    let mut out = String::with_capacity(bounded.len());
    let mut index = 0;
    while index < chars.len() {
        if chars[index].is_ascii_digit() {
            let start = index;
            let mut digits = 0usize;
            let mut end = index;
            while end < chars.len() {
                if chars[end].is_ascii_digit() {
                    digits += 1;
                    end += 1;
                } else if digits > 0 && matches!(chars[end], '+' | '-' | '.' | '(' | ')' | ' ') {
                    // Phone numbers and card-like sequences are often split by
                    // separators; treat them as one run.
                    end += 1;
                } else {
                    break;
                }
            }
            if digits >= 7 {
                out.push_str("[REDACTED]");
            } else {
                out.extend(chars[start..end].iter());
            }
            index = end;
        } else {
            out.push(chars[index]);
            index += 1;
        }
    }
    out.split_whitespace()
        .map(|word| {
            if word.contains('@') {
                "[REDACTED]"
            } else {
                word
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn downstream_operation(request: &EmailIngestRequest, maildir: &str) -> EmailOperationDto {
    // The output path is derived entirely from server-owned refs. It stays relative and is
    // confined to the tenant root by the core email handler, never chosen by the renderer.
    let suffix = format!(
        "{}_{}",
        request.source_ref.replace(':', "_"),
        request.mailbox_ref.replace(':', "_"),
    );
    let output = format!(".hacienda-out/{suffix}");
    let source = maildir.to_string();
    match request.operation {
        EmailIngestOperationDto::IndexForRag => EmailOperationDto::IndexForRag {
            source,
            index: None,
        },
        EmailIngestOperationDto::ExtractNer => EmailOperationDto::ExtractNER { source, output },
        EmailIngestOperationDto::Redact => EmailOperationDto::Redact { source, output },
        EmailIngestOperationDto::Pipeline => EmailOperationDto::Pipeline { source, output },
    }
}

/// SPEC REVIEW: TeamOnPremise does not support pseudonymization. A tenant-keyed
/// deterministic token in a redacted projection is an offline PII oracle unless
/// a full capability/reveal contract exists; the team viewer only reveals via a
/// short-lived, audited capability and cannot safely mint arbitrary tokens.
/// Accept `redact` only; the low-level `/v1/email/process` and LocalFull keep
/// their existing tenant-keyed behavior.
fn reject_pseudonymize(mode: EmailPrivacyDto) -> Result<(), ApiError> {
    match mode {
        EmailPrivacyDto::Redact => Ok(()),
        EmailPrivacyDto::Pseudonymize => Err(ApiError::invalid_request(
            "Pseudonymization is not supported for TeamOnPremise email ingestion.",
        )),
    }
}

fn downstream_request(request: &EmailIngestRequest, maildir: &str) -> EmailProcessRequest {
    EmailProcessRequest {
        backend: EmailBackendDto::Maildir {
            path: maildir.to_string(),
        },
        // Always redact on this surface: `reject_pseudonymize` guards the
        // handlers, and the mapping itself cannot turn into a token mode.
        privacy_mode: PrivacyModeDto {
            mode: PrivacyModeValue::Redact,
        },
        operation: downstream_operation(request, maildir),
        dossier_id: request.dossier_id.clone(),
    }
}

fn resolve_maildir(state: &ApiState, request: &EmailIngestRequest) -> Result<String, ApiError> {
    if !safe_ref(&request.source_ref) || !safe_ref(&request.mailbox_ref) {
        return Err(ApiError::invalid_request("Invalid email source reference."));
    }
    state
        .email_sources
        .maildir_for(&request.source_ref, &request.mailbox_ref)
        .ok_or_else(ApiError::not_found)
}

// ── Viewer projection ───────────────────────────────────────────────────────────────────

/// Resolve the capability-bound document in the tenant's redacted email corpus
/// and build a typed, content-safe viewer projection. The document is read from
/// the store only after the capability has been authorized and consumed, so no
/// clear text is ever sent to a renderer and a replayed capability finds nothing.
async fn project_viewer_content(
    state: &ApiState,
    ctx: &AuthContext,
    capability_ref: &str,
    record: &ViewerCapabilityRecord,
) -> Result<ViewerProjectionDto, ApiError> {
    let store = state.rag_store.as_ref().ok_or_else(ApiError::not_found)?;
    let tenant_ctx = Caller::Principal(ctx).tenant_ctx();
    let collection = scope_collection_name(&tenant_ctx, "email-corpus");

    // Both refs are one-way hashes; the only safe lookup is to walk the tenant's
    // own collection and compare. The walk is bounded so it cannot be used as a
    // resource-exhaustion primitive.
    const PAGE: u32 = 200;
    const MAX_DOCUMENTS: u32 = 10_000;
    let mut offset = 0u32;
    let mut target = None;
    while offset < MAX_DOCUMENTS {
        let (summaries, _) = store
            .list_documents(&collection, PAGE, offset)
            .await
            .map_err(|_| ApiError::not_found())?;
        if summaries.is_empty() {
            break;
        }
        for summary in &summaries {
            if opaque_ref(&summary.id.0) == record.document_ref
                && summary
                    .external_id
                    .as_deref()
                    .is_some_and(|id| opaque_ref(id) == record.email_ref)
            {
                target = Some(summary.id.clone());
                break;
            }
        }
        if target.is_some() {
            break;
        }
        offset = offset.saturating_add(summaries.len() as u32);
    }
    let document_id = target.ok_or_else(ApiError::not_found)?;

    let (document, chunks) = store
        .get_document_chunks(&collection, &document_id)
        .await
        .map_err(|_| ApiError::not_found())?
        .ok_or_else(ApiError::not_found)?;

    // A document may only be viewed by its owning tenant.
    if document
        .tenant_id
        .as_deref()
        .is_some_and(|tenant| tenant != ctx.tenant.as_str())
    {
        return Err(ApiError::forbidden());
    }

    let mut headers = Vec::new();
    for name in ["subject", "from", "to", "cc", "date", "message_id"] {
        if let Some(value) = document.metadata.get(name).and_then(|v| v.as_str()) {
            headers.push(ViewerHeaderDto {
                name: name.to_string(),
                token_only: safe_header_value(value),
            });
        }
    }

    let mut body = document.full_text.clone();
    for chunk in &chunks {
        body.push('\n');
        body.push_str(&chunk.content);
    }
    let body_token_only = safe_body(&body);

    let mut attachment_refs = Vec::new();
    if let Some(reference) = &record.attachment_ref {
        attachment_refs.push(reference.clone());
    }
    if let Some(values) = document
        .metadata
        .get("attachments")
        .and_then(|value| value.as_array())
    {
        for value in values.iter().filter_map(|value| value.as_str()) {
            let reference = opaque_ref(value);
            if !attachment_refs.contains(&reference) {
                attachment_refs.push(reference);
            }
        }
    }

    Ok(ViewerProjectionDto {
        capability_ref: capability_ref.to_string(),
        document_ref: record.document_ref.clone(),
        email_ref: record.email_ref.clone(),
        headers,
        body_token_only,
        attachment_refs,
        projection_kind: ViewerProjectionKindDto::RedactedTokenOnly,
    })
}

// ── Handlers ────────────────────────────────────────────────────────────────────────────

/// `GET /v1/team/email/sources` — list opaque source refs and their mailbox refs.
#[utoipa::path(
    get,
    path = "/v1/team/email/sources",
    tag = "team-email",
    operation_id = "listTeamEmailSources",
    description = "Requires documents:process",
    security(("bearerAuth" = [])),
    responses(
        (status = 200, description = "Opaque, server-provisioned sources and mailboxes", body = EmailSourcesResponse),
        (status = 401, description = "Authentication required"),
        (status = 403, description = "Caller lacks documents:process")
    )
)]
pub async fn list_email_sources(
    State(state): State<ApiState>,
    parts: Parts,
) -> Result<Json<EmailSourcesResponse>, ApiError> {
    authenticated(&parts)?;
    Ok(Json(EmailSourcesResponse {
        sources: state.email_sources.sources_dto(),
    }))
}

/// `POST /v1/team/email/ingest` — synchronous ingestion via server-provisioned refs.
#[utoipa::path(
    post,
    path = "/v1/team/email/ingest",
    tag = "team-email",
    operation_id = "ingestTeamEmail",
    description = "Requires documents:process",
    security(("bearerAuth" = [])),
    request_body(content = EmailIngestRequest, description = "Opaque source/mailbox refs only; no paths or credentials", content_type = "application/json"),
    responses(
        (status = 200, description = "Content-free email operation result"),
        (status = 400, description = "Invalid or insecure request body"),
        (status = 401, description = "Authentication required"),
        (status = 403, description = "Caller lacks documents:process")
    )
)]
pub async fn ingest_email(
    State(state): State<ApiState>,
    parts: Parts,
    SafeJson(request): SafeJson<EmailIngestRequest>,
) -> Result<Json<EmailOperationResultDto>, ApiError> {
    authenticated(&parts)?;
    reject_pseudonymize(request.privacy_mode)?;
    let maildir = resolve_maildir(&state, &request)?;
    let downstream = downstream_request(&request, &maildir);
    process_email(State(state), parts, SafeJson(downstream)).await
}

/// `POST /v1/team/email/ingest/async` — job-backed ingestion. The job stores the same
/// transport DTO as the synchronous route; no raw entity text or path is persisted.
#[utoipa::path(
    post,
    path = "/v1/team/email/ingest/async",
    tag = "team-email",
    operation_id = "ingestTeamEmailAsync",
    description = "Requires documents:process",
    security(("bearerAuth" = [])),
    request_body(content = EmailIngestRequest, description = "Opaque source/mailbox refs only; no paths or credentials", content_type = "application/json"),
    responses(
        (status = 202, description = "Job accepted; poll /v1/jobs/{id} and /v1/jobs/{id}/result", body = AsyncJobResponse),
        (status = 400, description = "Invalid or insecure request body"),
        (status = 401, description = "Authentication required"),
        (status = 403, description = "Caller lacks documents:process")
    )
)]
pub async fn ingest_email_async(
    State(state): State<ApiState>,
    parts: Parts,
    SafeJson(request): SafeJson<EmailIngestRequest>,
) -> Result<(StatusCode, Json<AsyncJobResponse>), ApiError> {
    authenticated(&parts)?;
    reject_pseudonymize(request.privacy_mode)?;
    let maildir = resolve_maildir(&state, &request)?;
    let downstream = downstream_request(&request, &maildir);
    process_email_async(State(state), parts, SafeJson(downstream)).await
}

/// `POST /v1/team/viewer/capability` — mint a short-lived, tenant/document/email-bound
/// capability. Requires `pii:reveal`; every input is an opaque reference.
#[utoipa::path(
    post,
    path = "/v1/team/viewer/capability",
    tag = "team-viewer",
    operation_id = "issueTeamViewerCapability",
    description = "Requires pii:reveal",
    security(("bearerAuth" = [])),
    request_body(content = ViewerCapabilityRequest, description = "Opaque document/email/attachment/token refs only", content_type = "application/json"),
    responses(
        (status = 200, description = "Short-lived, tenant/document/email-bound viewer capability", body = ViewerCapabilityDto),
        (status = 400, description = "Invalid viewer reference"),
        (status = 401, description = "Authentication required"),
        (status = 403, description = "Caller lacks pii:reveal")
    )
)]
pub async fn issue_viewer_capability(
    State(state): State<ApiState>,
    parts: Parts,
    SafeJson(request): SafeJson<ViewerCapabilityRequest>,
) -> Result<Json<ViewerCapabilityDto>, ApiError> {
    let ctx = authenticated(&parts)?;
    ctx.require_capability(Capability::PiiReveal)
        .map_err(|_| ApiError::forbidden())?;
    for reference in [&request.document_ref, &request.email_ref] {
        if !safe_ref(reference) {
            return Err(ApiError::invalid_request("Invalid viewer reference."));
        }
    }
    if !safe_token_ref(&request.token_ref) {
        return Err(ApiError::invalid_request("Invalid viewer reference."));
    }
    if request
        .attachment_ref
        .as_deref()
        .is_some_and(|reference| !safe_ref(reference))
    {
        return Err(ApiError::invalid_request("Invalid viewer reference."));
    }
    let capability =
        state
            .viewer_capabilities
            .issue(&request, ctx.tenant.as_str(), &ctx.principal_id);
    state.viewer_capabilities.audit_event(
        None,
        &ctx,
        "issue",
        "issued",
        &capability.capability_ref,
    );
    Ok(Json(capability))
}

/// `POST /v1/team/viewer/reveal` — perform a server-side reveal under an authorized
/// capability. The plaintext never enters a DTO, a job, or a log line.
#[utoipa::path(
    post,
    path = "/v1/team/viewer/reveal",
    tag = "team-viewer",
    operation_id = "revealTeamViewerContent",
    description = "Requires pii:reveal",
    security(("bearerAuth" = [])),
    request_body(content = ViewerActionRequest, description = "Opaque capability ref only", content_type = "application/json"),
    responses(
        (status = 200, description = "Content-free server-side reveal outcome", body = ViewerRevealResultDto),
        (status = 401, description = "Authentication required"),
        (status = 403, description = "Capability not bound to this tenant/principal")
    )
)]
pub async fn reveal_viewer_content(
    State(state): State<ApiState>,
    parts: Parts,
    SafeJson(request): SafeJson<ViewerActionRequest>,
) -> Result<Json<ViewerRevealResultDto>, ApiError> {
    let ctx = authenticated(&parts)?;
    ctx.require_capability(Capability::PiiReveal)
        .map_err(|_| ApiError::forbidden())?;
    if !safe_ref(&request.capability_ref) {
        return Err(ApiError::invalid_request("Invalid viewer capability."));
    }
    let record = state
        .viewer_capabilities
        .authorize(&request.capability_ref, &ctx, "reveal")?;
    let caller = Caller::Principal(&ctx);
    let revealed = state
        .facade
        .reveal_token_with_auth(caller, &record.token_ref)
        .await
        .is_ok();
    state.viewer_capabilities.audit_event(
        Some(&record),
        &ctx,
        "reveal",
        if revealed {
            "revealed_server_side"
        } else {
            "reveal_unavailable"
        },
        &request.capability_ref,
    );
    Ok(Json(ViewerRevealResultDto {
        capability_ref: request.capability_ref,
        revealed,
        document_ref: record.document_ref,
        email_ref: record.email_ref,
    }))
}

/// `POST /v1/team/viewer/open` — consume the capability and return a typed,
/// content-safe projection of the bound message. The capability is single-use;
/// replay, expiry and cross-tenant use are rejected and audited. No clear-text
/// email or attachment crosses this boundary.
#[utoipa::path(
    post,
    path = "/v1/team/viewer/open",
    tag = "team-viewer",
    operation_id = "openTeamViewerContent",
    description = "Requires pii:reveal",
    security(("bearerAuth" = [])),
    request_body(content = ViewerActionRequest, description = "Opaque capability ref only", content_type = "application/json"),
    responses(
        (status = 200, description = "Typed, redacted/token-only viewer projection", body = ViewerProjectionDto),
        (status = 401, description = "Authentication required"),
        (status = 403, description = "Capability not bound to this tenant/principal"),
        (status = 404, description = "Capability unknown, expired or already consumed")
    )
)]
pub async fn open_viewer_content(
    State(state): State<ApiState>,
    parts: Parts,
    SafeJson(request): SafeJson<ViewerActionRequest>,
) -> Result<Json<ViewerProjectionDto>, ApiError> {
    let ctx = authenticated(&parts)?;
    ctx.require_capability(Capability::PiiReveal)
        .map_err(|_| ApiError::forbidden())?;
    if !safe_ref(&request.capability_ref) {
        return Err(ApiError::invalid_request("Invalid viewer capability."));
    }
    let record = state
        .viewer_capabilities
        .consume(&request.capability_ref, &ctx, "open")?;
    project_viewer_content(&state, &ctx, &request.capability_ref, &record)
        .await
        .map(Json)
}

/// `GET /v1/team/viewer/audit` — content-free audit events for the caller's tenant only.
#[utoipa::path(
    get,
    path = "/v1/team/viewer/audit",
    tag = "team-viewer",
    operation_id = "getTeamViewerAudit",
    description = "Requires audit:read",
    security(("bearerAuth" = [])),
    responses(
        (status = 200, description = "Content-free audit events for the caller's tenant", body = ViewerAuditResponse),
        (status = 401, description = "Authentication required"),
        (status = 403, description = "Caller lacks audit:read")
    )
)]
pub async fn viewer_audit(
    State(state): State<ApiState>,
    parts: Parts,
) -> Result<Json<ViewerAuditResponse>, ApiError> {
    let ctx = authenticated(&parts)?;
    ctx.require_capability(Capability::AuditRead)
        .map_err(|_| ApiError::forbidden())?;
    Ok(Json(
        state
            .viewer_capabilities
            .audit_for_tenant(ctx.tenant.as_str()),
    ))
}

/// `POST /v1/team/email/search` — token-only email search over the shared, tenant-scoped
/// RAG store. Reuses the exact redaction path used by document retrieval.
#[utoipa::path(
    post,
    path = "/v1/team/email/search",
    tag = "team-email",
    operation_id = "searchTeamEmail",
    description = "Requires documents:process",
    security(("bearerAuth" = [])),
    request_body(content = EmailSearchRequest, description = "Opaque source/mailbox refs and a bounded query", content_type = "application/json"),
    responses(
        (status = 200, description = "Token-only, tenant-scoped email search hits", body = EmailSearchResponse),
        (status = 400, description = "Invalid or insecure request body"),
        (status = 401, description = "Authentication required"),
        (status = 403, description = "Caller lacks documents:process")
    )
)]
pub async fn search_email(
    State(state): State<ApiState>,
    parts: Parts,
    SafeJson(request): SafeJson<EmailSearchRequest>,
) -> Result<Json<EmailSearchResponse>, ApiError> {
    authenticated(&parts)?;
    resolve_maildir(
        &state,
        &EmailIngestRequest {
            source_ref: request.source_ref.clone(),
            mailbox_ref: request.mailbox_ref.clone(),
            operation: EmailIngestOperationDto::IndexForRag,
            privacy_mode: EmailPrivacyDto::default(),
            dossier_id: request.dossier_id.clone(),
        },
    )?;
    if request.query_text.trim().is_empty() {
        return Err(ApiError::invalid_request("Search query must not be empty."));
    }
    let top_k = request.top_k.unwrap_or(10).clamp(1, 50);
    let mut query = RetrieveQuery {
        mode: RetrieveMode::FullText,
        query_text: Some(request.query_text.clone()),
        top_k,
        include_content: true,
        include_document: true,
        dossier_id: request.dossier_id.clone(),
        ..RetrieveQuery::vector(top_k)
    };
    query.query_vector = None;
    let output = retrieve(
        State(state),
        parts,
        Path("email-corpus".to_string()),
        SafeJson(query),
    )
    .await?
    .0;
    let hits = output
        .chunks
        .into_iter()
        .map(|chunk| {
            let document = chunk.document.as_ref();
            EmailSearchHitDto {
                document_ref: opaque_ref(&chunk.document_id.0),
                external_ref: chunk
                    .external_id
                    .as_deref()
                    .map(opaque_ref)
                    .unwrap_or_default(),
                source_kind: EmailSourceKindDto::Email,
                mime: document.and_then(|doc| doc.mime.clone()),
                dossier_id: document.and_then(|doc| doc.dossier_id.clone()),
                version_ref: document.and_then(|doc| doc.version_ref.clone()),
                ordinal: chunk.ordinal,
                score: chunk.score,
                projection: chunk.content.unwrap_or_default(),
                viewer_policy: ViewerPolicyDto::CapabilityRequired,
            }
        })
        .collect();
    Ok(Json(EmailSearchResponse { hits }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use hacienda_core::auth::{AuthExtension, CapabilitySet};
    use hacienda_core::tenancy::TenantId;

    fn ctx(tenant: &str, principal: &str, caps: &[Capability]) -> Arc<AuthContext> {
        Arc::new(AuthContext::with_tenant(
            principal,
            TenantId::new(tenant),
            CapabilitySet::new(caps.iter().copied()),
        ))
    }

    fn parts_for(auth: &Arc<AuthContext>) -> Parts {
        let (mut parts, _) = axum::http::Request::builder()
            .body(())
            .expect("building an empty request cannot fail")
            .into_parts();
        parts.extensions.insert(AuthExtension(Arc::clone(auth)));
        parts
    }

    /// The ingest body must not accept any path/credential/raw-text carrier.
    #[test]
    fn ingest_dto_rejects_paths_credentials_and_raw_text() {
        let base = r#"{"source_ref":"default","mailbox_ref":"inbox","operation":{"operation":"index_for_rag"}}"#;
        assert!(serde_json::from_str::<EmailIngestRequest>(base).is_ok());
        for (field, body) in [
            (
                "backend",
                base.replace('}', r#","backend":{"type":"maildir","path":"/etc"}}"#),
            ),
            ("path", base.replace('}', r#","path":"/etc"}"#)),
            ("token", base.replace('}', r#","token":"oauth-secret"}"#)),
            ("password", base.replace('}', r#","password":"hunter2"}"#)),
            ("server", base.replace('}', r#","server":"evil.example"}"#)),
            (
                "token_command",
                base.replace('}', r#","token_command":["sh"]}"#),
            ),
            ("text", base.replace('}', r#","text":"raw message body"}"#)),
            (
                "message",
                base.replace('}', r#","message":"raw message body"}"#),
            ),
        ] {
            let result = serde_json::from_str::<EmailIngestRequest>(&body);
            assert!(
                result.is_err(),
                "insecure field `{field}` was accepted: {body}"
            );
        }
    }

    /// The viewer capability request must not accept a plaintext/credential carrier
    /// either — every field is an opaque reference.
    #[test]
    fn viewer_dto_rejects_plaintext_and_path_carriers() {
        let base = r#"{"document_ref":"doc1","email_ref":"msg1","token_ref":"tok1"}"#;
        assert!(serde_json::from_str::<ViewerCapabilityRequest>(base).is_ok());
        for body in [
            base.replace('}', r#","value":"jean.dupont@example.com"}"#),
            base.replace('}', r#","path":"/srv/mail/inbox"}"#),
            base.replace('}', r#","password":"secret"}"#),
            base.replace('}', r#","raw_text":"raw message"}"#),
        ] {
            assert!(
                serde_json::from_str::<ViewerCapabilityRequest>(&body).is_err(),
                "insecure viewer body accepted: {body}"
            );
        }
    }

    /// A capability is opaque (no tenant/email/path embedded) and short-lived.
    #[test]
    fn capability_is_opaque_and_bounded() {
        let store = ViewerCapabilityStore::new(Duration::from_secs(60));
        let request: ViewerCapabilityRequest = serde_json::from_str(
            r#"{"document_ref":"doc1","email_ref":"msg1","token_ref":"tok1"}"#,
        )
        .unwrap();
        let dto = store.issue(&request, "acme", "alice");
        assert!(dto.capability_ref.starts_with("vcap_"));
        for secret in ["acme", "alice", "doc1", "msg1", "tok1", "/", "mail"] {
            assert!(
                !dto.capability_ref.contains(secret),
                "capability leaked `{secret}`"
            );
        }
        assert!(store
            .authorize(&dto.capability_ref, &ctx("acme", "alice", &[]), "open")
            .is_ok());
    }

    /// Cross-tenant and cross-principal use of a capability is denied and audited.
    #[test]
    fn capability_denied_across_tenants_and_principals() {
        let store = ViewerCapabilityStore::new(Duration::from_secs(60));
        let request: ViewerCapabilityRequest = serde_json::from_str(
            r#"{"document_ref":"doc1","email_ref":"msg1","token_ref":"tok1"}"#,
        )
        .unwrap();
        let issued = store.issue(&request, "acme", "alice");
        for (tenant, principal) in [("globex", "alice"), ("acme", "bob")] {
            let err = store
                .authorize(
                    &issued.capability_ref,
                    &ctx(tenant, principal, &[]),
                    "reveal",
                )
                .expect_err("foreign use must be denied");
            assert_eq!(err.code, crate::error::ApiErrorCode::Forbidden);
        }
        let audit = store.audit_for_tenant("globex");
        assert_eq!(audit.events.len(), 1);
        assert_eq!(audit.events[0].outcome, "denied");
        assert_eq!(audit.events[0].action, "reveal");
        assert_eq!(audit.events[0].capability_ref, issued.capability_ref);
        // Each tenant sees only the denial raised by its own principal. acme's
        // auditor sees bob's cross-principal attempt, never globex's alice event.
        let acme_audit = store.audit_for_tenant("acme");
        assert_eq!(acme_audit.events.len(), 1);
        assert_eq!(acme_audit.events[0].outcome, "denied");
        // The denial is recorded under the *caller's* identity, not the capability
        // owner's: acme's auditor sees bob (the foreign principal), and globex's
        // auditor sees alice. Cross-tenant denial still returns Forbidden above.
        assert_eq!(acme_audit.events[0].tenant, "acme");
        assert_eq!(acme_audit.events[0].principal, "bob");
        assert_eq!(audit.events[0].principal, "alice");
        assert_eq!(audit.events[0].tenant, "globex");
    }

    /// An expired capability is no longer usable.
    #[test]
    fn capability_expires() {
        let store = ViewerCapabilityStore::new(Duration::from_millis(1));
        let request: ViewerCapabilityRequest = serde_json::from_str(
            r#"{"document_ref":"doc1","email_ref":"msg1","token_ref":"tok1"}"#,
        )
        .unwrap();
        let issued = store.issue(&request, "acme", "alice");
        std::thread::sleep(Duration::from_millis(20));
        assert!(store
            .authorize(&issued.capability_ref, &ctx("acme", "alice", &[]), "reveal")
            .is_err());
    }

    /// `open` consumes a capability: a second use is a replay, rejected with a
    /// generic 404 and recorded in the content-free audit trail.
    #[test]
    fn capability_is_single_use_and_replay_is_audited() {
        let store = ViewerCapabilityStore::new(Duration::from_secs(60));
        let request: ViewerCapabilityRequest = serde_json::from_str(
            r#"{"document_ref":"doc1","email_ref":"msg1","token_ref":"[EMAIL:k1:AAAA]"}"#,
        )
        .unwrap();
        let issued = store.issue(&request, "acme", "alice");
        assert!(store
            .consume(&issued.capability_ref, &ctx("acme", "alice", &[]), "open")
            .is_ok());
        let replay = store
            .consume(&issued.capability_ref, &ctx("acme", "alice", &[]), "open")
            .expect_err("a consumed capability must not be reusable");
        assert_eq!(replay.code, crate::error::ApiErrorCode::NotFound);
        let audit = store.audit_for_tenant("acme");
        assert!(audit
            .events
            .iter()
            .any(|event| event.outcome == "consumed_projection"));
        assert!(audit.events.iter().any(|event| event.outcome == "replayed"));
        // A cross-principal replay is denied and audited, never served.
        let cross = store
            .consume(&issued.capability_ref, &ctx("acme", "bob", &[]), "open")
            .expect_err("a foreign principal must not replay a consumed capability");
        assert_eq!(cross.code, crate::error::ApiErrorCode::NotFound);
    }

    /// Projection helpers must never emit clear PII, even if a store row was
    /// populated with attacker-controlled header/body values.
    #[test]
    fn projection_helpers_are_content_safe() {
        let header = safe_header_value("Re: jean.dupont@example.com [EMAIL:k1:ABC]");
        assert!(header.contains("[EMAIL:k1:ABC]"));
        assert!(header.contains("[REDACTED]"));
        assert!(!header.contains("jean.dupont@example.com"));
        assert!(!header.contains("Re:"));

        let body = safe_body("Bonjour [PERSON:k1:DEF]\njean.dupont@example.com +33 6 12 34 56 78");
        assert!(body.contains("[PERSON:k1:DEF]"));
        assert!(!body.contains("jean.dupont@example.com"));
        assert!(!body.contains("+33 6 12 34 56 78"));
    }

    /// The source registry never accepts an absolute path or traversal from config.
    #[test]
    fn source_registry_rejects_absolute_paths() {
        let registry = EmailSourceRegistry::from_parts(vec![(
            "default".to_string(),
            vec![("inbox".to_string(), "/etc/passwd".to_string())],
        )]);
        assert!(registry.sources_dto().is_empty());
        let registry = EmailSourceRegistry::from_parts(vec![(
            "default".to_string(),
            vec![("inbox".to_string(), "../victim".to_string())],
        )]);
        assert!(registry.sources_dto().is_empty());
        let registry = EmailSourceRegistry::from_parts(vec![(
            "default".to_string(),
            vec![("inbox".to_string(), "INBOX".to_string())],
        )]);
        assert_eq!(registry.sources_dto()[0].mailbox_refs, vec!["inbox"]);
    }

    /// Renderer DTOs carry no raw email/attachment PII carrier by construction.
    #[test]
    fn renderer_dtos_have_no_raw_content_carrier() {
        let dto = ViewerCapabilityDto {
            capability_ref: "vcap_x".to_string(),
            expires_at: "2026-01-01T00:00:00Z".to_string(),
            document_ref: "ref_doc".to_string(),
            email_ref: "ref_email".to_string(),
            attachment_ref: None,
            delivery: ViewerDeliveryDto::ServerSide,
        };
        let value = serde_json::to_value(&dto).unwrap();
        let keys: Vec<&str> = value
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        for forbidden in [
            "text",
            "body",
            "subject",
            "from",
            "to",
            "path",
            "password",
            "token",
            "oauth",
            "raw",
            "content",
            "message",
            "attachment_data",
        ] {
            assert!(!keys.contains(&forbidden), "renderer DTO has `{forbidden}`");
        }
        let reveal = ViewerRevealResultDto {
            capability_ref: "vcap_x".to_string(),
            revealed: true,
            document_ref: "ref_doc".to_string(),
            email_ref: "ref_email".to_string(),
        };
        let value = serde_json::to_value(&reveal).unwrap();
        let keys: Vec<&str> = value
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        assert!(!keys.contains(&"value"));
        assert!(!keys.contains(&"plaintext"));
        assert!(!keys.contains(&"text"));
    }

    /// `parts_for` must install an authenticated context — the guard all handlers rely on.
    #[test]
    fn authenticated_parts_are_recognized() {
        let auth = ctx("acme", "alice", &[]);
        let parts = parts_for(&auth);
        assert_eq!(authenticated(&parts).unwrap().principal_id, "alice");
    }
}
