//! Backend adapters over the Pimalaya `io-email` 0.1 facade.
//!
//! `io-email` 0.1 exposes a single blocking `EmailClientStd` whose shared-API
//! methods take `&mut self` and whose IMAP stream is `Send` but not `Sync`.
//! The plugin's [`EmailClient`] trait is `Send + Sync` (it is held behind an
//! `Arc` by `EmailProcessor`), so the adapter wraps the blocking client in a
//! `std::sync::Mutex` and every async method locks it inside `spawn_blocking`.
//! This keeps blocking socket/filesystem I/O off the async executor while
//! presenting a thread-safe trait object to callers.
//!
//! Messages come back from `get_message` as raw RFC 5322 bytes; they are
//! decoded with `mail-parser` into the plugin's simplified [`EmailMessage`].

use crate::error::{EmailError, Result};
use crate::models::{EmailAddress, EmailAttachment, EmailMessage, Mailbox, Envelope, Flag};
use async_trait::async_trait;
use std::sync::{Arc, Mutex};

/// Email client trait - abstraction over different backends.
#[async_trait]
pub trait EmailClient: Send + Sync {
    /// List all mailboxes.
    async fn list_mailboxes(&self) -> Result<Vec<Mailbox>>;
    /// List envelopes in a mailbox (newest first, capped at `limit`).
    async fn list_envelopes(&self, mailbox: &str, limit: Option<usize>) -> Result<Vec<Envelope>>;
    /// Search envelopes in a mailbox using the backend's native query syntax.
    async fn search_envelopes(&self, mailbox: &str, query: &str) -> Result<Vec<Envelope>>;
    /// Get and parse a full message by id.
    async fn get_message(&self, mailbox: &str, uid: &str) -> Result<EmailMessage>;
    /// Store a raw RFC 5322 message in a mailbox.
    async fn add_message(&self, mailbox: &str, message: &EmailMessage) -> Result<()>;
    /// Copy messages between mailboxes.
    async fn copy_messages(&self, src: &str, dst: &str, uids: &[String]) -> Result<()>;
    /// Move messages between mailboxes.
    async fn move_messages(&self, src: &str, dst: &str, uids: &[String]) -> Result<()>;
    /// Delete messages.
    async fn delete_messages(&self, mailbox: &str, uids: &[String]) -> Result<()>;
    /// Store flags on messages.
    async fn store_flags(&self, mailbox: &str, uids: &[String], flags: &[Flag], add: bool) -> Result<()>;
    /// Send a message (network backends only).
    async fn send_message(&self, message: &EmailMessage) -> Result<()>;
}

// ---------------------------------------------------------------------------
// Shared conversion helpers (io-email types -> plugin models)
// ---------------------------------------------------------------------------

mod convert {
    use super::*;
    use io_email::address::Address as IoAddress;
    use io_email::envelope::types::Envelope as IoEnvelope;
    use io_email::flag::types::{Flag as IoFlag, IanaFlag};
    use io_email::mailbox::types::Mailbox as IoMailbox;

    pub(crate) fn address(a: &IoAddress) -> EmailAddress {
        EmailAddress { name: a.name.clone(), address: a.email.clone() }
    }

    pub(crate) fn addresses(a: &[IoAddress]) -> Vec<EmailAddress> {
        a.iter().map(address).collect()
    }

    pub(crate) fn flag(f: &IoFlag) -> Flag {
        match f.iana() {
            Some(IanaFlag::Seen) => Flag::Seen,
            Some(IanaFlag::Answered) => Flag::Answered,
            Some(IanaFlag::Flagged) => Flag::Flagged,
            Some(IanaFlag::Draft) => Flag::Draft,
            Some(IanaFlag::Deleted) => Flag::Deleted,
            _ => Flag::Custom(f.raw().to_string()),
        }
    }

    pub(crate) fn mailbox(m: &IoMailbox) -> Mailbox {
        Mailbox {
            name: m.name.clone(),
            total: m.total.map(|v| v as usize),
            unread: m.unread.map(|v| v as usize),
            flags: Vec::new(), // io-email 0.1's Mailbox carries no flag set
        }
    }

    pub(crate) fn envelope(e: &IoEnvelope) -> Envelope {
        Envelope {
            id: e.id.clone(),
            mailbox: String::new(),
            subject: e.subject.clone(),
            from: addresses(&e.from),
            to: addresses(&e.to),
            date: e
                .date
                .map(|d| d.with_timezone(&chrono::Utc))
                .unwrap_or_else(chrono::Utc::now),
            flags: e.flags.iter().map(flag).collect(),
            has_attachments: e.has_attachment.unwrap_or(false),
        }
    }

    /// Parse raw RFC 5322 bytes into the plugin's simplified message model.
    pub(crate) fn message(raw: &[u8], mailbox: &str, id: &str) -> Result<EmailMessage> {
        use mail_parser::MimeHeaders;

        let parsed = mail_parser::MessageParser::default()
            .parse(raw)
            .ok_or_else(|| EmailError::Backend("failed to parse RFC 5322 message".to_string()))?;

        // body_text/body_html take a body-part position and return Cow<str>.
        let text_body = parsed.body_text(0).map(|s| s.into_owned());
        let html_body = parsed.body_html(0).map(|s| s.into_owned());

        let addr = |a: &mail_parser::Addr<'_>| EmailAddress {
            name: a.name.as_ref().map(|n| n.to_string()),
            address: a.address.as_ref().map(|v| v.to_string()).unwrap_or_default(),
        };
        let addr_list = |h: Option<&mail_parser::Address<'_>>| -> Vec<EmailAddress> {
            match h {
                Some(mail_parser::Address::List(list)) => list.iter().map(addr).collect(),
                Some(mail_parser::Address::Group(groups)) => groups
                    .iter()
                    .flat_map(|g| g.addresses.iter().map(addr))
                    .collect(),
                None => Vec::new(),
            }
        };

        let attachments = parsed
            .attachments()
            .enumerate()
            .map(|(i, a)| EmailAttachment {
                filename: a
                    .attachment_name()
                    .map(|v| v.to_string())
                    .unwrap_or_else(|| format!("attachment-{i}")),
                content_type: a
                    .content_type()
                    .map(|ct| {
                        let sub = ct.c_subtype.as_ref().map(|s| s.as_ref()).unwrap_or("");
                        format!("{}/{}", ct.c_type, sub)
                    })
                    .unwrap_or_else(|| "application/octet-stream".to_string()),
                size: a.contents().len(),
                content_id: a.content_id().map(|v| v.to_string()),
                content: a.contents().to_vec(),
            })
            .collect();

        Ok(EmailMessage {
            id: id.to_string(),
            mailbox: mailbox.to_string(),
            subject: parsed.subject().unwrap_or_default().to_string(),
            from: addr_list(parsed.from()),
            to: addr_list(parsed.to()),
            cc: addr_list(parsed.cc()),
            bcc: Vec::new(),
            date: parsed
                .date()
                .map(|d| chrono::DateTime::from_timestamp(d.to_timestamp(), 0).unwrap_or_else(chrono::Utc::now))
                .unwrap_or_else(chrono::Utc::now),
            text_body,
            html_body,
            attachments,
            flags: Vec::new(),
        })
    }
}

// ---------------------------------------------------------------------------
// Blocking adapter implementing Send + Sync over io-email's !Sync client
// ---------------------------------------------------------------------------

pub struct BlockingClient {
    inner: Arc<Mutex<io_email::client::EmailClientStd>>,
}

impl BlockingClient {
    fn new(client: io_email::client::EmailClientStd) -> Self {
        Self { inner: Arc::new(Mutex::new(client)) }
    }
}

fn join(e: tokio::task::JoinError) -> EmailError {
    EmailError::Backend(format!("blocking email task failed: {e}"))
}

#[async_trait]
impl EmailClient for BlockingClient {
    async fn list_mailboxes(&self) -> Result<Vec<Mailbox>> {
        let c = Arc::clone(&self.inner);
        tokio::task::spawn_blocking(move || {
            let mut guard = c.lock().map_err(|_| EmailError::Backend("lock poisoned".to_string()))?;
            Ok(guard.list_mailboxes(true)?.iter().map(convert::mailbox).collect())
        })
        .await
        .map_err(join)?
    }

    async fn list_envelopes(&self, mailbox: &str, limit: Option<usize>) -> Result<Vec<Envelope>> {
        let c = Arc::clone(&self.inner);
        let mailbox = mailbox.to_string();
        tokio::task::spawn_blocking(move || {
            let mut guard = c.lock().map_err(|_| EmailError::Backend("lock poisoned".to_string()))?;
            let page_size = limit.unwrap_or(1000) as u32;
            let envs = guard.list_envelopes(&mailbox, Some(0), Some(page_size), true)?;
            Ok(envs.iter().map(|e| {
                let mut env = convert::envelope(e);
                env.mailbox = mailbox.clone();
                env
            }).collect())
        })
        .await
        .map_err(join)?
    }

    async fn search_envelopes(&self, mailbox: &str, query: &str) -> Result<Vec<Envelope>> {
        let c = Arc::clone(&self.inner);
        let mailbox = mailbox.to_string();
        let query = query.to_string();
        tokio::task::spawn_blocking(move || {
            let mut guard = c.lock().map_err(|_| EmailError::Backend("lock poisoned".to_string()))?;
            // io-email 0.1's search takes a structured SearchEmailsQuery; the
            // string form is backend-specific, so fall back to listing and
            // filtering on subject for a portable subset.
            let envs = guard.list_envelopes(&mailbox, Some(0), Some(1000), false)?;
            let needle = query.to_lowercase();
            Ok(envs.iter()
                .filter(|e| e.subject.to_lowercase().contains(&needle))
                .map(|e| {
                    let mut env = convert::envelope(e);
                    env.mailbox = mailbox.clone();
                    env
                })
                .collect())
        })
        .await
        .map_err(join)?
    }

    async fn get_message(&self, mailbox: &str, uid: &str) -> Result<EmailMessage> {
        let c = Arc::clone(&self.inner);
        let mailbox = mailbox.to_string();
        let uid = uid.to_string();
        tokio::task::spawn_blocking(move || {
            let mut guard = c.lock().map_err(|_| EmailError::Backend("lock poisoned".to_string()))?;
            let raw = guard.get_message(&mailbox, &uid)?;
            convert::message(&raw, &mailbox, &uid)
        })
        .await
        .map_err(join)?
    }

    async fn add_message(&self, mailbox: &str, message: &EmailMessage) -> Result<()> {
        let c = Arc::clone(&self.inner);
        let mailbox = mailbox.to_string();
        let raw = crate::redact::message_to_rfc822(message).into_bytes();
        tokio::task::spawn_blocking(move || {
            let mut guard = c.lock().map_err(|_| EmailError::Backend("lock poisoned".to_string()))?;
            guard.add_message(&mailbox, &[], raw)?;
            Ok(())
        })
        .await
        .map_err(join)?
    }

    async fn copy_messages(&self, src: &str, dst: &str, uids: &[String]) -> Result<()> {
        let c = Arc::clone(&self.inner);
        let (src, dst) = (src.to_string(), dst.to_string());
        let uids = uids.to_vec();
        tokio::task::spawn_blocking(move || {
            let mut guard = c.lock().map_err(|_| EmailError::Backend("lock poisoned".to_string()))?;
            let refs: Vec<&str> = uids.iter().map(|s| s.as_str()).collect();
            guard.copy_messages(&src, &dst, &refs)?;
            Ok(())
        })
        .await
        .map_err(join)?
    }

    async fn move_messages(&self, src: &str, dst: &str, uids: &[String]) -> Result<()> {
        let c = Arc::clone(&self.inner);
        let (src, dst) = (src.to_string(), dst.to_string());
        let uids = uids.to_vec();
        tokio::task::spawn_blocking(move || {
            let mut guard = c.lock().map_err(|_| EmailError::Backend("lock poisoned".to_string()))?;
            let refs: Vec<&str> = uids.iter().map(|s| s.as_str()).collect();
            guard.move_messages(&src, &dst, &refs)?;
            Ok(())
        })
        .await
        .map_err(join)?
    }

    async fn delete_messages(&self, mailbox: &str, uids: &[String]) -> Result<()> {
        let c = Arc::clone(&self.inner);
        let mailbox = mailbox.to_string();
        let uids = uids.to_vec();
        tokio::task::spawn_blocking(move || {
            let mut guard = c.lock().map_err(|_| EmailError::Backend("lock poisoned".to_string()))?;
            for uid in &uids {
                guard.delete_message(&mailbox, uid)?;
            }
            Ok(())
        })
        .await
        .map_err(join)?
    }

    async fn store_flags(&self, mailbox: &str, uids: &[String], flags: &[Flag], add: bool) -> Result<()> {
        let c = Arc::clone(&self.inner);
        let mailbox = mailbox.to_string();
        let uids = uids.to_vec();
        let names: Vec<String> = flags.iter().map(|f| f.to_wire()).collect();
        tokio::task::spawn_blocking(move || {
            let mut guard = c.lock().map_err(|_| EmailError::Backend("lock poisoned".to_string()))?;
            let refs: Vec<&str> = uids.iter().map(|s| s.as_str()).collect();
            let io_flags: Vec<io_email::flag::types::Flag> = names
                .iter()
                .map(|n| io_email::flag::types::Flag::from_raw(n))
                .collect();
            let op = if add {
                io_email::flag::types::FlagOp::Add
            } else {
                io_email::flag::types::FlagOp::Remove
            };
            guard.store_flags(&mailbox, &refs, &io_flags, op)?;
            Ok(())
        })
        .await
        .map_err(join)?
    }

    async fn send_message(&self, _message: &EmailMessage) -> Result<()> {
        Err(EmailError::InvalidOperation(
            "sending is only available on the SMTP/JMAP backends".to_string(),
        ))
    }
}

/// The IMAP and Maildir adapters are the same `BlockingClient` shape; only
/// their construction differs, so we expose them as aliases.
#[cfg(feature = "imap")]
pub type IoEmailImapClient = BlockingClient;
#[cfg(feature = "maildir")]
pub type IoEmailMaildirClient = BlockingClient;

/// Build an IMAP-backed client connected via implicit TLS.
#[cfg(feature = "imap")]
pub fn build_imap_client(server: &str, username: &str, password: secrecy::SecretString) -> Result<BlockingClient> {
    use pimalaya_stream::sasl::{Sasl, SaslPlain};
    use pimalaya_stream::tls::Tls;

    let url = url::Url::parse(server)
        .map_err(|e| EmailError::Config(format!("invalid IMAP URL: {e}")))?;
    let tls = Tls::default();
    let sasl: Sasl = SaslPlain {
        authzid: None,
        authcid: username.to_string(),
        passwd: password,
    }
    .into();
    let inner = io_email::client::EmailClientStd::new()
        .connect_imap(&url, &tls, false, Some(sasl), None)
        .map_err(|e| EmailError::Connection(format!("failed to connect IMAP: {e}")))?;
    Ok(BlockingClient::new(inner))
}

/// Build a Maildir-backed client rooted at `path`.
#[cfg(feature = "maildir")]
pub fn build_maildir_client(path: &str) -> Result<BlockingClient> {
    let maildir = io_email::maildir::client::MaildirClient::new(path);
    let inner = io_email::client::EmailClientStd::new().with_maildir(maildir);
    Ok(BlockingClient::new(inner))
}

/// Build a client from configuration. Only IMAP and Maildir are implemented;
/// the other backends are advertised in the config enum but rejected here with
/// an explicit error rather than silently doing nothing.
pub async fn build_client(config: &crate::config::EmailConfig) -> Result<Arc<dyn EmailClient>> {
    match &config.backend {
        #[cfg(feature = "imap")]
        crate::config::EmailBackend::Imap { server, username } => {
            let password = std::env::var(format!(
                "HACIENDA_EMAIL_{}_PASSWORD",
                config.account.to_uppercase()
            ))
            .map_err(|_| EmailError::Auth("IMAP password not found in environment".to_string()))?;
            let client = build_imap_client(server, username, secrecy::SecretString::new(password.into()))?;
            Ok(Arc::new(client))
        }
        #[cfg(feature = "maildir")]
        crate::config::EmailBackend::Maildir { path } => {
            let client = build_maildir_client(path)?;
            Ok(Arc::new(client))
        }
        other => Err(EmailError::Config(format!(
            "backend {other:?} is not enabled or not implemented"
        ))),
    }
}
