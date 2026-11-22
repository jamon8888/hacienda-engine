//! Native LocalFull email session service.
//!
//! This is the in-process `hacienda-email` surface for a trusted desktop host
//! (the Tauri LocalFull profile). It deliberately has **no** HTTP or renderer
//! dependency: every parameter that can name a file, a credential, or clear
//! content is supplied by the host here and never crosses IPC.
//!
//! Security contract (P0.5):
//!
//! * The [`PiiEngine`] is injected by the host and is built from the **same**
//!   facade / key resolver / OS keychain that document PII and the local viewer
//!   use. This module never constructs a facade, resolves an environment key,
//!   or generates a second secret.
//! * Every value returned to the renderer is an opaque ref or a token-only
//!   projection. Absolute paths, OAuth tokens, passwords, display names and raw
//!   message/attachment text stay in this process.
//! * A reveal is only possible through a short-lived, one-shot, audited,
//!   message-bound [`ViewerCapability`](MessageBoundViewerCapability). The
//!   plaintext is handed exclusively to the injected [`NativeMessageViewer`],
//!   which opens the original inside the trusted native viewer; it is never
//!   returned to the caller.
//! * Metadata tokens are minted under a tenant-qualified category domain. With
//!   a single shared key this is what makes a token from tenant B fail the
//!   cross-tenant check in tenant A's viewer capability (the ciphertext AAD
//!   differs), even though both tenants share one keychain key.

use crate::backend::{build_client, EmailClient};
use crate::config::EmailBackend;
use crate::pii::PiiEngine;
use crate::{EmailConfig, EmailOperation, EmailProcessor, EmailRequest, EmailSource};
use hacienda_core::audit::{AuditEntryInput, AuditStore, EntitySource, RedactionAction};
use hacienda_core::auth::{AuthContext, Caller, Capability, CapabilitySet};
use hacienda_core::tenancy::TenantId;
use hacienda_rag::{RagStore, RetrieveQuery};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Mutex;
use zeroize::Zeroizing;

/// Lifetime of a message-bound reveal/open capability.
pub const VIEWER_CAPABILITY_TTL: Duration = Duration::from_secs(60);
/// Email chunks live in this collection; LocalFull scopes retrieval by tenant.
pub const EMAIL_RAG_COLLECTION: &str = "email-corpus";
const NATIVE_VIEWER_PREFIX: &str = "email-viewer-";
const AUDIT_CONFIG_HASH: &str = "hacienda-secrecy-client-email-native-v1";

/// Content-free error codes. Safe to serialize across IPC: no path, token or
/// message text is ever echoed, and all reveal failures collapse to one code.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NativeEmailError {
    InvalidRequest,
    UnknownReference,
    Unavailable,
    Unauthorized,
    Expired,
    Replayed,
    /// A token or capability from a different tenant was presented.
    CrossTenant,
    /// The token is not one of the opaque tokens projected for this message.
    TokenNotBound,
    AuditFailed,
}

impl std::fmt::Display for NativeEmailError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::InvalidRequest => "email request is invalid",
            Self::UnknownReference => "email reference is unknown",
            Self::Unavailable => "email service is unavailable",
            Self::Unauthorized => "viewer capability is not authorized",
            Self::Expired => "viewer capability has expired",
            Self::Replayed => "viewer capability was already used",
            Self::CrossTenant => "viewer capability belongs to another tenant",
            Self::TokenNotBound => "token is not bound to this message",
            Self::AuditFailed => "email viewer audit failed closed",
        })
    }
}

impl std::error::Error for NativeEmailError {}

type Result<T> = std::result::Result<T, NativeEmailError>;

/// The trusted sink for a revealed original. The Tauri implementation opens
/// the RFC822 file in the native viewer and never serializes `revealed` to the
/// renderer; tests use a recording implementation to assert what was disclosed.
pub trait NativeMessageViewer: Send + Sync {
    /// Open one original in the native viewer.
    ///
    /// `revealed` contains `(token, plaintext)` pairs that the message-bound
    /// capability authorized for this one reveal. It is `Zeroizing` so the
    /// plaintext is wiped when the viewer drops it.
    fn open_message(
        &self,
        path: &Path,
        revealed: &[(String, Zeroizing<String>)],
    ) -> std::result::Result<(), ()>;
}

/// Host-supplied registration data. Never sent by the renderer: `maildir_root`
/// is the absolute path of the provisioned local mailbox.
#[derive(Debug, Clone)]
pub struct RegisterAccountRequest {
    pub tenant: String,
    pub principal: String,
    pub maildir_root: PathBuf,
    pub label: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct AccountProjection {
    pub account_ref: String,
    pub label_token_only: String,
    pub mailbox_count: usize,
}

#[derive(Clone, Debug, Serialize)]
pub struct MailboxProjection {
    pub mailbox_ref: String,
    pub account_ref: String,
    /// Opaque token for the mailbox name, never the name itself.
    pub name_token_only: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct MessageProjection {
    pub message_ref: String,
    pub mailbox_ref: String,
    pub account_ref: String,
    /// Subject, display names and addresses as tenant-keyed tokens only.
    pub envelope_token_only: serde_json::Value,
}

#[derive(Clone, Debug, Serialize)]
pub struct JobProjection {
    pub job_ref: String,
    pub account_ref: String,
    pub status: &'static str,
    pub messages_processed: usize,
    pub chunks_persisted: usize,
    pub errors: usize,
}

#[derive(Clone, Debug, Serialize)]
pub struct EmailSearchHit {
    pub message_ref: Option<String>,
    pub content_token_only: String,
    pub ordinal: u32,
    pub score: f32,
}

#[derive(Clone, Debug, Serialize)]
pub struct ViewerCapabilityProjection {
    pub capability_ref: String,
    pub message_ref: String,
    pub viewer_ref: String,
    pub expires_in_secs: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ViewerOpenResponse {
    pub opened: bool,
}

/// A minted, unexpired, unconsumed reveal/open authority. Never serialized.
#[derive(Debug)]
struct MessageBoundViewerCapability {
    message_ref: String,
    tenant: TenantId,
    viewer_label: String,
    expires_at: Instant,
    consumed: bool,
}

#[derive(Clone)]
struct MailboxSlot {
    mailbox_ref: String,
    /// Name reported by the backend; only its token is ever projected.
    name: String,
    /// Logical path accepted by the backend's `list_envelopes`. io-email's
    /// Maildir listing reports the root spool by its directory name, while its
    /// envelope listing addresses that same spool with an empty path.
    logical: String,
}

#[derive(Clone)]
struct AccountSession {
    tenant: TenantId,
    label_token_only: String,
    caller_ctx: AuthContext,
    config: EmailConfig,
    client: Arc<dyn EmailClient>,
    /// Absolute Maildir root. Kept native; never projected.
    root: PathBuf,
    mailboxes: Vec<MailboxSlot>,
}

#[derive(Clone)]
struct MessageSlot {
    account_ref: String,
    tenant: TenantId,
    mailbox: String,
    uid: String,
    /// Absolute original RFC822 path. Kept native; never projected.
    original_path: PathBuf,
    token_domains: HashMap<String, String>,
}

/// The LocalFull email session service. Constructed once by the host and kept
/// in Tauri managed state; all registries are keyed by opaque references.
pub struct EmailSessionService {
    pii: PiiEngine,
    rag: Arc<dyn RagStore>,
    audit: Arc<dyn AuditStore>,
    viewer: Arc<dyn NativeMessageViewer>,
    accounts: Mutex<HashMap<String, AccountSession>>,
    mailboxes: Mutex<HashMap<String, String>>,
    messages: Mutex<HashMap<String, MessageSlot>>,
    capabilities: Mutex<HashMap<String, MessageBoundViewerCapability>>,
    jobs: Mutex<HashMap<String, JobProjection>>,
}

impl EmailSessionService {
    pub fn new(
        pii: PiiEngine,
        rag: Arc<dyn RagStore>,
        audit: Arc<dyn AuditStore>,
        viewer: Arc<dyn NativeMessageViewer>,
    ) -> Self {
        Self {
            pii,
            rag,
            audit,
            viewer,
            accounts: Mutex::new(HashMap::new()),
            mailboxes: Mutex::new(HashMap::new()),
            messages: Mutex::new(HashMap::new()),
            capabilities: Mutex::new(HashMap::new()),
            jobs: Mutex::new(HashMap::new()),
        }
    }

    /// Register a natively provisioned account. The renderer never supplies a
    /// path or a credential here; the host resolves both before calling.
    pub async fn register_account(
        &self,
        request: RegisterAccountRequest,
    ) -> Result<AccountProjection> {
        let root = request
            .maildir_root
            .canonicalize()
            .map_err(|_| NativeEmailError::InvalidRequest)?;
        if !root.is_dir() {
            return Err(NativeEmailError::InvalidRequest);
        }
        let tenant_raw = if request.tenant.trim().is_empty() {
            "local_full"
        } else {
            request.tenant.as_str()
        };
        let tenant = TenantId::new(normalize_tenant(tenant_raw));
        let caller_ctx = AuthContext::with_tenant(
            request.principal,
            tenant.clone(),
            CapabilitySet::new([Capability::DocumentsProcess, Capability::PiiReveal]),
        );
        let config = EmailConfig {
            backend: EmailBackend::Maildir {
                path: root.to_string_lossy().into_owned(),
            },
            // RAG rows are stamped with the account/tenant, never the display
            // label, so retrieval scoping matches the caller's tenant.
            account: tenant.as_str().to_owned(),
            ..EmailConfig::default()
        };
        let client = build_client(&config)
            .await
            .map_err(|_| NativeEmailError::Unavailable)?;
        let caller = Caller::Principal(&caller_ctx);
        let label_token_only = self
            .pii
            .project_metadata(caller, &token_domain(&tenant, "account"), &request.label)
            .await
            .map_err(|_| NativeEmailError::Unavailable)?;
        let raw_mailboxes = client
            .list_mailboxes()
            .await
            .map_err(|_| NativeEmailError::Unavailable)?;
        let account_ref = opaque_ref("acct");
        let mut mailboxes = Vec::with_capacity(raw_mailboxes.len());
        {
            let mut public = self.mailboxes.lock().await;
            for mailbox in raw_mailboxes {
                let name = if mailbox.name.trim().is_empty() {
                    "INBOX".to_owned()
                } else {
                    mailbox.name
                };
                // Probe the logical names the backend accepts for this spool.
                // The root Maildir is addressed with an empty path, not with
                // the directory name reported by `list_mailboxes`.
                let mut logical = String::new();
                for candidate in [name.as_str(), "", "INBOX"] {
                    if let Ok(envelopes) = client.list_envelopes(candidate, Some(1)).await {
                        if !envelopes.is_empty() {
                            logical = candidate.to_owned();
                            break;
                        }
                    }
                }
                let mailbox_ref = opaque_ref("mbox");
                public.insert(mailbox_ref.clone(), account_ref.clone());
                mailboxes.push(MailboxSlot {
                    mailbox_ref,
                    name,
                    logical,
                });
            }
        }
        let mailbox_count = mailboxes.len();
        self.accounts.lock().await.insert(
            account_ref.clone(),
            AccountSession {
                tenant,
                label_token_only: label_token_only.clone(),
                caller_ctx,
                config,
                client,
                root,
                mailboxes,
            },
        );
        Ok(AccountProjection {
            account_ref,
            label_token_only,
            mailbox_count,
        })
    }

    /// Opaque account refs and token-only labels for host-provisioned accounts.
    /// The renderer can select an account but never learns a path or credential.
    pub async fn accounts(&self) -> Vec<AccountProjection> {
        self.accounts
            .lock()
            .await
            .iter()
            .map(|(account_ref, account)| AccountProjection {
                account_ref: account_ref.clone(),
                label_token_only: account.label_token_only.clone(),
                mailbox_count: account.mailboxes.len(),
            })
            .collect()
    }

    /// Token-only mailbox list for a registered account.
    pub async fn list_mailboxes(&self, account_ref: &str) -> Result<Vec<MailboxProjection>> {
        let account = self.account(account_ref).await?;
        let caller_ctx = account.caller_ctx.clone();
        let tenant = account.tenant.clone();
        let slots = account.mailboxes.clone();
        drop(account);
        let caller = Caller::Principal(&caller_ctx);
        let mut out = Vec::with_capacity(slots.len());
        for slot in slots {
            let name_token_only = self
                .pii
                .project_metadata(caller, &token_domain(&tenant, "mailbox"), &slot.name)
                .await
                .map_err(|_| NativeEmailError::Unavailable)?;
            out.push(MailboxProjection {
                mailbox_ref: slot.mailbox_ref,
                account_ref: account_ref.to_owned(),
                name_token_only,
            });
        }
        Ok(out)
    }

    /// Token-only envelope list. Every PII-bearing field is minted as a
    /// tenant-qualified token before it is placed in the projection.
    pub async fn list_messages(
        &self,
        account_ref: &str,
        mailbox_ref: &str,
    ) -> Result<Vec<MessageProjection>> {
        let account = self.account(account_ref).await?;
        let tenant = account.tenant.clone();
        let mailbox_name = account
            .mailboxes
            .iter()
            .find(|slot| slot.mailbox_ref == mailbox_ref)
            .map(|slot| slot.logical.clone())
            .ok_or(NativeEmailError::UnknownReference)?;
        let client = Arc::clone(&account.client);
        let caller_ctx = account.caller_ctx.clone();
        let root = account.root.clone();
        drop(account);
        let caller = Caller::Principal(&caller_ctx);
        let envelopes = client
            .list_envelopes(&mailbox_name, Some(1000))
            .await
            .map_err(|_| NativeEmailError::Unavailable)?;
        let mut out = Vec::with_capacity(envelopes.len());
        for envelope in envelopes {
            let mut token_domains = HashMap::new();
            let subject = self
                .mint(caller, &tenant, "subject", &envelope.subject, &mut token_domains)
                .await?;
            let from = self
                .address_tokens(caller, &tenant, &envelope.from, &mut token_domains)
                .await?;
            let to = self
                .address_tokens(caller, &tenant, &envelope.to, &mut token_domains)
                .await?;
            let envelope_token_only = serde_json::json!({
                "subject": subject,
                "from": from,
                "to": to,
                "has_attachments": envelope.has_attachments,
            });
            let original_path = maildir_original_path(&root, &mailbox_name, &envelope.id);
            let message_ref = opaque_ref("msg");
            self.messages.lock().await.insert(
                message_ref.clone(),
                MessageSlot {
                    account_ref: account_ref.to_owned(),
                    tenant: tenant.clone(),
                    mailbox: mailbox_name.clone(),
                    uid: envelope.id.clone(),
                    original_path,
                    token_domains,
                },
            );
            out.push(MessageProjection {
                message_ref,
                mailbox_ref: mailbox_ref.to_owned(),
                account_ref: account_ref.to_owned(),
                envelope_token_only,
            });
        }
        Ok(out)
    }

    /// Ingest one mailbox into the shared RAG store. The returned job carries
    /// only counters and an opaque reference.
    pub async fn ingest(&self, account_ref: &str, mailbox_ref: &str) -> Result<JobProjection> {
        let account = self.account(account_ref).await?;
        let mailbox_name = account
            .mailboxes
            .iter()
            .find(|slot| slot.mailbox_ref == mailbox_ref)
            .map(|slot| slot.logical.clone())
            .ok_or(NativeEmailError::UnknownReference)?;
        let config = account.config.clone();
        let caller_ctx = account.caller_ctx.clone();
        let dossier = account.tenant.as_str().to_owned();
        drop(account);
        let processor = EmailProcessor::new(config, self.pii.clone())
            .await
            .map_err(|_| NativeEmailError::Unavailable)?
            .with_rag_store(Arc::clone(&self.rag))
            .with_dossier_id(dossier);
        let response = processor
            .process(
                EmailRequest {
                    operation: EmailOperation::IndexForRag {
                        source: EmailSource::Account {
                            name: account_ref.to_owned(),
                            mailbox: Some(mailbox_name),
                        },
                        index_name: Some(EMAIL_RAG_COLLECTION.to_owned()),
                        chunk_size: None,
                    },
                    config_overrides: None,
                },
                Caller::Principal(&caller_ctx),
            )
            .await
            .map_err(|_| NativeEmailError::Unavailable)?;
        let job = JobProjection {
            job_ref: opaque_ref("job"),
            account_ref: account_ref.to_owned(),
            status: if response.success { "completed" } else { "failed" },
            messages_processed: response.processed_count,
            chunks_persisted: response
                .rag_stats
                .as_ref()
                .map(|stats| stats.chunks_created)
                .unwrap_or(0),
            errors: response.errors.len(),
        };
        self.jobs.lock().await.insert(job.job_ref.clone(), job.clone());
        Ok(job)
    }

    pub async fn job_status(&self, job_ref: &str) -> Result<JobProjection> {
        self.jobs
            .lock()
            .await
            .get(job_ref)
            .cloned()
            .ok_or(NativeEmailError::UnknownReference)
    }

    /// Retrieve token-only email snippets. The query is redacted locally before
    /// it reaches the store and every stored chunk is re-redacted before it is
    /// returned across the native boundary.
    pub async fn search(&self, account_ref: &str, raw_query: &str) -> Result<Vec<EmailSearchHit>> {
        let account = self.account(account_ref).await?;
        let tenant = account.tenant.clone();
        let caller_ctx = account.caller_ctx.clone();
        drop(account);
        let caller = Caller::Principal(&caller_ctx);
        let (tokenized_query, _) = self
            .pii
            .redact(caller, raw_query, &[], "", false)
            .await
            .map_err(|_| NativeEmailError::Unavailable)?;
        let mut query = RetrieveQuery::vector(8);
        // Email chunks are dimension-0 BYOV in the shared store: a full-text
        // query would not match, so retrieve by tenant with an empty vector and
        // re-check the tokenized text client-side.
        query.query_vector = Some(Vec::new());
        query.tenant_id = Some(tenant.as_str().to_owned());
        query.include_content = true;
        query.include_document = true;
        let output = self
            .rag
            .retrieve(EMAIL_RAG_COLLECTION, &query)
            .await
            .map_err(|_| NativeEmailError::Unavailable)?;
        let messages = self.messages.lock().await;
        let mut hits = Vec::with_capacity(output.chunks.len());
        for chunk in output.chunks {
            let Some(content) = chunk.content else { continue };
            let (content_token_only, _) = self
                .pii
                .redact(caller, &content, &[], "", false)
                .await
                .map_err(|_| NativeEmailError::Unavailable)?;
            if !content_token_only
                .to_lowercase()
                .contains(&tokenized_query.to_lowercase())
            {
                continue;
            }
            let external = chunk
                .document
                .as_ref()
                .and_then(|document| document.external_id.as_deref())
                .or(chunk.external_id.as_deref());
            let message_ref = external
                .and_then(|external| resolve_message_ref(&messages, account_ref, external));
            hits.push(EmailSearchHit {
                message_ref,
                content_token_only,
                ordinal: chunk.ordinal,
                score: chunk.score,
            });
        }
        Ok(hits)
    }

    /// Issue a short-lived, one-shot capability bound to one message and one
    /// viewer window. Audited with content-free data only.
    pub async fn authorize_message_view(
        &self,
        message_ref: &str,
        viewer_label: &str,
    ) -> Result<ViewerCapabilityProjection> {
        if !viewer_label.starts_with(NATIVE_VIEWER_PREFIX)
            && !viewer_label.starts_with("local-viewer-")
        {
            return Err(NativeEmailError::Unauthorized);
        }
        let message = self.message(message_ref).await?;
        if !message.original_path.is_file() {
            return Err(NativeEmailError::Unavailable);
        }
        let capability_ref = opaque_ref("cap");
        self.capabilities.lock().await.insert(
            capability_ref.clone(),
            MessageBoundViewerCapability {
                message_ref: message_ref.to_owned(),
                tenant: message.tenant.clone(),
                viewer_label: viewer_label.to_owned(),
                expires_at: Instant::now() + VIEWER_CAPABILITY_TTL,
                consumed: false,
            },
        );
        let account_ref = message.account_ref.clone();
        let digest = blake3::hash(
            format!("{capability_ref}:{message_ref}:{account_ref}:{viewer_label}").as_bytes(),
        )
        .to_hex()
        .to_string();
        self.append_audit(&message.tenant, "email_message_view", digest)
            .await?;
        Ok(ViewerCapabilityProjection {
            capability_ref,
            message_ref: message_ref.to_owned(),
            viewer_ref: opaque_ref("viewer"),
            expires_in_secs: VIEWER_CAPABILITY_TTL.as_secs(),
        })
    }

    /// Reveal the authorized tokens **inside the native viewer** and open the
    /// original. The plaintext is passed only to [`NativeMessageViewer`]; it is
    /// never returned to the renderer or to the caller of this method.
    pub async fn reveal_and_open(
        &self,
        capability_ref: &str,
        message_ref: &str,
        viewer_label: &str,
        token: &str,
    ) -> Result<ViewerOpenResponse> {
        let (domain, tenant) = {
            let mut capabilities = self.capabilities.lock().await;
            let capability = capabilities
                .get_mut(capability_ref)
                .ok_or(NativeEmailError::Unauthorized)?;
            if capability.consumed {
                return Err(NativeEmailError::Replayed);
            }
            if Instant::now() >= capability.expires_at {
                capabilities.remove(capability_ref);
                return Err(NativeEmailError::Expired);
            }
            if capability.viewer_label != viewer_label || capability.message_ref != message_ref {
                return Err(NativeEmailError::Unauthorized);
            }
            let message = self.message(message_ref).await?;
            if message.tenant != capability.tenant {
                return Err(NativeEmailError::CrossTenant);
            }
            if capability.tenant != message.tenant {
                return Err(NativeEmailError::CrossTenant);
            }
            let domain = message
                .token_domains
                .get(token)
                .ok_or(NativeEmailError::TokenNotBound)?
                .clone();
            // The token's category carries the tenant-qualified domain as AAD;
            // a token minted for another tenant cannot pass this check even if
            // the shared OS key would technically decrypt it.
            if !token_starts_with_domain(token, &domain) {
                return Err(NativeEmailError::CrossTenant);
            }
            (domain, message.tenant.clone())
        };
        let caller_ctx = self
            .accounts
            .lock()
            .await
            .values()
            .find(|account| account.tenant == tenant)
            .map(|account| account.caller_ctx.clone())
            .ok_or(NativeEmailError::UnknownReference)?;
        let plaintext = self
            .pii
            .facade()
            .reveal_token_with_auth(Caller::Principal(&caller_ctx), token)
            .await
            .map_err(|_| NativeEmailError::Unauthorized)?;
        let revealed = vec![(token.to_owned(), Zeroizing::new(plaintext))];
        let message = self.message(message_ref).await?;
        let path = message.original_path.clone();
        // The native viewer call is blocking (it launches the OS viewer). Move
        // it off the async runtime so a slow viewer cannot stall a worker or
        // delay other email commands sharing this service. The viewer handle,
        // path and revealed plaintext move into the task as owned values; the
        // plaintext is dropped inside that task and never crosses back to the
        // async caller, the renderer or any log surface.
        let viewer = Arc::clone(&self.viewer);
        tokio::task::spawn_blocking(move || viewer.open_message(&path, &revealed))
            .await
            .map_err(|_| NativeEmailError::Unavailable)?
            .map_err(|_| NativeEmailError::Unavailable)?;
        if let Some(capability) = self.capabilities.lock().await.get_mut(capability_ref) {
            capability.consumed = true;
        }
        let digest =
            blake3::hash(format!("{capability_ref}:{message_ref}:{domain}").as_bytes())
                .to_hex()
                .to_string();
        self.append_audit(&tenant, "email_message_reveal", digest).await?;
        Ok(ViewerOpenResponse { opened: true })
    }

    async fn mint(
        &self,
        caller: Caller<'_>,
        tenant: &TenantId,
        field: &str,
        value: &str,
        domains: &mut HashMap<String, String>,
    ) -> Result<String> {
        let domain = token_domain(tenant, field);
        let token = self
            .pii
            .project_metadata(caller, &domain, value)
            .await
            .map_err(|_| NativeEmailError::Unavailable)?;
        domains.insert(token.clone(), domain);
        Ok(token)
    }

    async fn address_tokens(
        &self,
        caller: Caller<'_>,
        tenant: &TenantId,
        addresses: &[crate::models::EmailAddress],
        domains: &mut HashMap<String, String>,
    ) -> Result<Vec<serde_json::Value>> {
        let mut out = Vec::with_capacity(addresses.len());
        for address in addresses {
            let name = match address.name.as_deref().filter(|name| !name.is_empty()) {
                Some(name) => Some(self.mint(caller, tenant, "name", name, domains).await?),
                None => None,
            };
            let token = self
                .mint(caller, tenant, "address", &address.address, domains)
                .await?;
            out.push(serde_json::json!({ "name": name, "address": token }));
        }
        Ok(out)
    }

    async fn append_audit(&self, tenant: &TenantId, category: &str, digest: String) -> Result<()> {
        self.audit
            .append(
                tenant,
                vec![AuditEntryInput {
                    id: opaque_ref("audit"),
                    category: category.to_owned(),
                    action: RedactionAction::Reveal,
                    span_hash: digest,
                    span_length: 0,
                    confidence: None,
                    source: EntitySource::Manual,
                    pipeline_version: "hacienda-email-native-v1".to_owned(),
                    config_hash: AUDIT_CONFIG_HASH.to_owned(),
                    principal: None,
                    vertical: None,
                    model: None,
                }],
            )
            .await
            .map_err(|_| NativeEmailError::AuditFailed)?;
        Ok(())
    }

    async fn account(&self, account_ref: &str) -> Result<AccountSession> {
        self.accounts
            .lock()
            .await
            .get(account_ref)
            .cloned()
            .ok_or(NativeEmailError::UnknownReference)
    }

    async fn message(&self, message_ref: &str) -> Result<MessageSlot> {
        self.messages
            .lock()
            .await
            .get(message_ref)
            .cloned()
            .ok_or(NativeEmailError::UnknownReference)
    }
}

/// Normalize a tenant id for use in a pseudonym category label. The OS key
/// resolver ignores tenant, so this is about token domain separation, not key
/// lookup; keeping it `[a-z0-9_]` also matches hacienda-core's tenant rules.
fn normalize_tenant(tenant: &str) -> String {
    tenant
        .chars()
        .map(|c| {
            if c.is_ascii_lowercase() || c.is_ascii_digit() {
                c
            } else {
                '_'
            }
        })
        .collect()
}

/// `PiiEngine::project_metadata` renders the category as `email.<domain>` and
/// `Pseudonymiser` uppercases it into the token label / AEAD AAD. The tenant
/// component makes identical clear values mint different tokens per tenant.
fn token_domain(tenant: &TenantId, field: &str) -> String {
    format!("{}.{}", tenant.as_str(), field)
}

fn token_starts_with_domain(token: &str, domain: &str) -> bool {
    let Some(label) = token
        .strip_prefix('[')
        .and_then(|body| body.split_once(':').map(|(label, _)| label))
    else {
        return false;
    };
    label.eq_ignore_ascii_case(&format!("email.{domain}"))
}

fn opaque_ref(prefix: &str) -> String {
    let digest = blake3::hash(format!("{prefix}:{}", uuid::Uuid::new_v4()).as_bytes()).to_hex();
    format!("{prefix}_{}", &digest[..24])
}

fn resolve_message_ref(
    messages: &HashMap<String, MessageSlot>,
    account_ref: &str,
    external_id: &str,
) -> Option<String> {
    let rest = external_id.strip_prefix("email:")?;
    // Chunk rows append `:chunk:<ordinal>` to the document external_id.
    let rest = rest.split(":chunk:").next().unwrap_or(rest);
    let mut parts = rest.splitn(3, ':');
    let _account = parts.next()?;
    let mailbox = parts.next().unwrap_or("");
    let uid = parts.next().unwrap_or("");
    messages
        .iter()
        .find(|(_, message)| {
            message.account_ref == account_ref
                && (message.uid == uid || uid.ends_with(&message.uid))
                && (message.mailbox == mailbox
                    || mailbox.is_empty()
                    || message.mailbox.is_empty())
        })
        .map(|(message_ref, _)| message_ref.clone())
}

/// Resolve the original RFC822 file inside the provisioned Maildir. Only
/// `cur/` and `new/` are considered, and the path never leaves the native
/// boundary. Maildir filenames carry `:2,<flags>` suffixes, so match by prefix.
fn maildir_original_path(root: &Path, mailbox: &str, uid: &str) -> PathBuf {
    // io-email reports the local spool as "INBOX" while the files live at the
    // Maildir root; other mailboxes are subdirectories of that root.
    let mut bases = vec![root.to_path_buf()];
    if !mailbox.is_empty() && !mailbox.eq_ignore_ascii_case("inbox") {
        bases.push(root.join(mailbox));
    }
    for base in bases {
        for subdir in ["cur", "new"] {
            let dir = base.join(subdir);
            let Ok(entries) = std::fs::read_dir(&dir) else { continue };
            for entry in entries.flatten() {
                let name = entry.file_name();
                let name = name.to_string_lossy();
                if name == uid || name.starts_with(&format!("{uid}:")) {
                    return entry.path();
                }
            }
        }
    }
    root.join(mailbox).join("new").join(uid)
}
