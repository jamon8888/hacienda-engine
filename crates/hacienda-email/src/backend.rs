use crate::error::{EmailError, Result};
use crate::models::{EmailAddress, EmailAttachment, EmailMessage, Mailbox, Envelope, Flag};
use async_trait::async_trait;
use std::sync::Arc;

/// Email client trait - abstraction over different backends
#[async_trait]
pub trait EmailClient: Send + Sync {
    /// List all mailboxes
    async fn list_mailboxes(&self) -> Result<Vec<Mailbox>>;

    /// List envelopes in a mailbox
    async fn list_envelopes(&self, mailbox: &str, limit: Option<usize>) -> Result<Vec<Envelope>>;

    /// Search envelopes in a mailbox
    async fn search_envelopes(&self, mailbox: &str, query: &str) -> Result<Vec<Envelope>>;

    /// Get full message by ID
    async fn get_message(&self, mailbox: &str, uid: &str) -> Result<EmailMessage>;

    /// Add a message to a mailbox
    async fn add_message(&self, mailbox: &str, message: &EmailMessage) -> Result<()>;

    /// Copy messages between mailboxes
    async fn copy_messages(&self, src: &str, dst: &str, uids: &[String]) -> Result<()>;

    /// Move messages between mailboxes
    async fn move_messages(&self, src: &str, dst: &str, uids: &[String]) -> Result<()>;

    /// Delete messages
    async fn delete_messages(&self, mailbox: &str, uids: &[String]) -> Result<()>;

    /// Store flags on messages
    async fn store_flags(&self, mailbox: &str, uids: &[String], flags: &[Flag], add: bool) -> Result<()>;

    /// Send a message
    async fn send_message(&self, message: &EmailMessage) -> Result<()>;
}

/// io-email std client wrapper
#[cfg(feature = "imap")]
pub mod imap_client {
    use super::*;
    use io_email::client::EmailClientStd;
    use pimalaya_stream::tls::Tls;
    use pimalaya_stream::sasl::Sasl as ImapSasl;
    use secrecy::SecretString;
    use url::Url;

    pub struct IoEmailImapClient {
        inner: EmailClientStd,
    }

    impl IoEmailImapClient {
        pub fn new(server: &str, username: &str, password: SecretString) -> Result<Self> {
            let url = Url::parse(server).map_err(|e| EmailError::Config(format!("Invalid IMAP URL: {}", e)))?;
            let tls = Tls::default();
            let sasl: ImapSasl = ImapSasl::Plain {
                username: username.into(),
                password,
            };

            let mut client = EmailClientStd::new();
            client = client.connect_imap(&url, &tls, false, Some(sasl), None)
                .map_err(|e| EmailError::Connection(format!("Failed to connect IMAP: {}", e)))?;

            Ok(Self { inner: client })
        }
    }

    #[async_trait]
    impl EmailClient for IoEmailImapClient {
        async fn list_mailboxes(&self) -> Result<Vec<Mailbox>> {
            let client = self.inner.clone();
            tokio::task::spawn_blocking(move || {
                client.list_mailboxes(true)
                    .map_err(|e| EmailError::Backend(e.to_string()))
                    .map(|mbs| mbs.into_iter().map(|mb| Mailbox {
                        name: mb.name,
                        total: mb.total,
                        unread: mb.unread,
                        flags: Vec::new(),
                    }).collect())
            }).await?
        }

        async fn list_envelopes(&self, mailbox: &str, limit: Option<usize>) -> Result<Vec<Envelope>> {
            let client = self.inner.clone();
            let mailbox = mailbox.to_string();
            tokio::task::spawn_blocking(move || {
                let lim = limit.unwrap_or(100);
                client.list_envelopes(&mailbox, Some(lim))
                    .map_err(|e| EmailError::Backend(e.to_string()))
                    .map(|envs| envs.into_iter().map(|e| Envelope {
                        id: e.id.to_string(),
                        mailbox: mailbox.clone(),
                        subject: e.subject.unwrap_or_default(),
                        from: e.from.into_iter().map(|a| EmailAddress {
                            name: a.name,
                            address: a.address,
                        }).collect(),
                        to: e.to.into_iter().map(|a| EmailAddress {
                            name: a.name,
                            address: a.address,
                        }).collect(),
                        date: e.date.unwrap_or_else(chrono::Utc::now),
                        flags: Vec::new(),
                        has_attachments: e.has_attachments,
                    }).collect())
            }).await?
        }

        async fn search_envelopes(&self, mailbox: &str, query: &str) -> Result<Vec<Envelope>> {
            let client = self.inner.clone();
            let mailbox = mailbox.to_string();
            let query = query.to_string();
            tokio::task::spawn_blocking(move || {
                client.search_envelopes(&mailbox, &query)
                    .map_err(|e| EmailError::Backend(e.to_string()))
                    .map(|envs| envs.into_iter().map(|e| Envelope {
                        id: e.id.to_string(),
                        mailbox: mailbox.clone(),
                        subject: e.subject.unwrap_or_default(),
                        from: e.from.into_iter().map(|a| EmailAddress {
                            name: a.name,
                            address: a.address,
                        }).collect(),
                        to: e.to.into_iter().map(|a| EmailAddress {
                            name: a.name,
                            address: a.address,
                        }).collect(),
                        date: e.date.unwrap_or_else(chrono::Utc::now),
                        flags: Vec::new(),
                        has_attachments: e.has_attachments,
                    }).collect())
            }).await?
        }

        async fn get_message(&self, mailbox: &str, uid: &str) -> Result<EmailMessage> {
            let client = self.inner.clone();
            let mailbox = mailbox.to_string();
            let uid = uid.to_string();
            tokio::task::spawn_blocking(move || {
                client.get_message(&mailbox, &uid)
                    .map_err(|e| EmailError::Backend(e.to_string()))
                    .map(|msg| EmailMessage {
                        id: msg.id.to_string(),
                        mailbox: mailbox.clone(),
                        subject: msg.subject.unwrap_or_default(),
                        from: msg.from.into_iter().map(|a| EmailAddress {
                            name: a.name,
                            address: a.address,
                        }).collect(),
                        to: msg.to.into_iter().map(|a| EmailAddress {
                            name: a.name,
                            address: a.address,
                        }).collect(),
                        cc: msg.cc.into_iter().map(|a| EmailAddress {
                            name: a.name,
                            address: a.address,
                        }).collect(),
                        bcc: Vec::new(),
                        date: msg.date.unwrap_or_else(chrono::Utc::now),
                        text_body: msg.text_body,
                        html_body: msg.html_body,
                        attachments: msg.attachments.into_iter().map(|a| EmailAttachment {
                            filename: a.filename.unwrap_or_default(),
                            content_type: a.content_type.unwrap_or_default(),
                            size: a.size.unwrap_or(0),
                            content_id: a.content_id,
                        }).collect(),
                        flags: Vec::new(),
                    })
            }).await?
        }

        async fn add_message(&self, mailbox: &str, message: &EmailMessage) -> Result<()> {
            let client = self.inner.clone();
            let mailbox = mailbox.to_string();
            tokio::task::spawn_blocking(move || {
                Ok(())
            }).await?
        }

        async fn copy_messages(&self, src: &str, dst: &str, uids: &[String]) -> Result<()> {
            let client = self.inner.clone();
            let src = src.to_string();
            let dst = dst.to_string();
            let uids = uids.to_vec();
            tokio::task::spawn_blocking(move || {
                client.copy_messages(&src, &dst, &uids)
                    .map_err(|e| EmailError::Backend(e.to_string()))
            }).await?
        }

        async fn move_messages(&self, src: &str, dst: &str, uids: &[String]) -> Result<()> {
            let client = self.inner.clone();
            let src = src.to_string();
            let dst = dst.to_string();
            let uids = uids.to_vec();
            tokio::task::spawn_blocking(move || {
                client.move_messages(&src, &dst, &uids)
                    .map_err(|e| EmailError::Backend(e.to_string()))
            }).await?
        }

        async fn delete_messages(&self, mailbox: &str, uids: &[String]) -> Result<()> {
            let client = self.inner.clone();
            let mailbox = mailbox.to_string();
            let uids = uids.to_vec();
            tokio::task::spawn_blocking(move || {
                client.delete_messages(&mailbox, &uids)
                    .map_err(|e| EmailError::Backend(e.to_string()))
            }).await?
        }

        async fn store_flags(&self, mailbox: &str, uids: &[String], flags: &[Flag], add: bool) -> Result<()> {
            let client = self.inner.clone();
            let mailbox = mailbox.to_string();
            let uids = uids.to_vec();
            tokio::task::spawn_blocking(move || {
                Ok(())
            }).await?
        }

        async fn send_message(&self, message: &EmailMessage) -> Result<()> {
            let client = self.inner.clone();
            tokio::task::spawn_blocking(move || {
                Ok(())
            }).await?
        }
    }
}

/// Maildir client wrapper
#[cfg(feature = "maildir")]
pub mod maildir_client {
    use super::*;
    use io_email::client::EmailClientStd;
    use io_maildir::client::MaildirClient;

    pub struct IoEmailMaildirClient {
        inner: EmailClientStd,
    }

    impl IoEmailMaildirClient {
        pub fn new(path: &str) -> Result<Self> {
            let maildir_client = MaildirClient::new(path);
            let mut client = EmailClientStd::new();
            client = client.with_maildir(maildir_client);

            Ok(Self { inner: client })
        }
    }

    #[async_trait]
    impl EmailClient for IoEmailMaildirClient {
        async fn list_mailboxes(&self) -> Result<Vec<Mailbox>> {
            let client = self.inner.clone();
            tokio::task::spawn_blocking(move || {
                client.list_mailboxes(true)
                    .map_err(|e| EmailError::Backend(e.to_string()))
                    .map(|mbs| mbs.into_iter().map(|mb| Mailbox {
                        name: mb.name,
                        total: mb.total,
                        unread: mb.unread,
                        flags: Vec::new(),
                    }).collect())
            }).await?
        }

        async fn list_envelopes(&self, mailbox: &str, limit: Option<usize>) -> Result<Vec<Envelope>> {
            let client = self.inner.clone();
            let mailbox = mailbox.to_string();
            tokio::task::spawn_blocking(move || {
                let lim = limit.unwrap_or(100);
                client.list_envelopes(&mailbox, Some(lim))
                    .map_err(|e| EmailError::Backend(e.to_string()))
                    .map(|envs| envs.into_iter().map(|e| Envelope {
                        id: e.id.to_string(),
                        mailbox: mailbox.clone(),
                        subject: e.subject.unwrap_or_default(),
                        from: e.from.into_iter().map(|a| EmailAddress {
                            name: a.name,
                            address: a.address,
                        }).collect(),
                        to: e.to.into_iter().map(|a| EmailAddress {
                            name: a.name,
                            address: a.address,
                        }).collect(),
                        date: e.date.unwrap_or_else(chrono::Utc::now),
                        flags: Vec::new(),
                        has_attachments: e.has_attachments,
                    }).collect())
            }).await?
        }

        async fn search_envelopes(&self, mailbox: &str, query: &str) -> Result<Vec<Envelope>> {
            let client = self.inner.clone();
            let mailbox = mailbox.to_string();
            let query = query.to_string();
            tokio::task::spawn_blocking(move || {
                client.search_envelopes(&mailbox, &query)
                    .map_err(|e| EmailError::Backend(e.to_string()))
                    .map(|envs| envs.into_iter().map(|e| Envelope {
                        id: e.id.to_string(),
                        mailbox: mailbox.clone(),
                        subject: e.subject.unwrap_or_default(),
                        from: e.from.into_iter().map(|a| EmailAddress {
                            name: a.name,
                            address: a.address,
                        }).collect(),
                        to: e.to.into_iter().map(|a| EmailAddress {
                            name: a.name,
                            address: a.address,
                        }).collect(),
                        date: e.date.unwrap_or_else(chrono::Utc::now),
                        flags: Vec::new(),
                        has_attachments: e.has_attachments,
                    }).collect())
            }).await?
        }

        async fn get_message(&self, mailbox: &str, uid: &str) -> Result<EmailMessage> {
            let client = self.inner.clone();
            let mailbox = mailbox.to_string();
            let uid = uid.to_string();
            tokio::task::spawn_blocking(move || {
                client.get_message(&mailbox, &uid)
                    .map_err(|e| EmailError::Backend(e.to_string()))
                    .map(|msg| EmailMessage {
                        id: msg.id.to_string(),
                        mailbox: mailbox.clone(),
                        subject: msg.subject.unwrap_or_default(),
                        from: msg.from.into_iter().map(|a| EmailAddress {
                            name: a.name,
                            address: a.address,
                        }).collect(),
                        to: msg.to.into_iter().map(|a| EmailAddress {
                            name: a.name,
                            address: a.address,
                        }).collect(),
                        cc: msg.cc.into_iter().map(|a| EmailAddress {
                            name: a.name,
                            address: a.address,
                        }).collect(),
                        bcc: Vec::new(),
                        date: msg.date.unwrap_or_else(chrono::Utc::now),
                        text_body: msg.text_body,
                        html_body: msg.html_body,
                        attachments: msg.attachments.into_iter().map(|a| EmailAttachment {
                            filename: a.filename.unwrap_or_default(),
                            content_type: a.content_type.unwrap_or_default(),
                            size: a.size.unwrap_or(0),
                            content_id: a.content_id,
                        }).collect(),
                        flags: Vec::new(),
                    })
            }).await?
        }

        async fn add_message(&self, mailbox: &str, message: &EmailMessage) -> Result<()> {
            let client = self.inner.clone();
            let mailbox = mailbox.to_string();
            tokio::task::spawn_blocking(move || {
                Ok(())
            }).await?
        }

        async fn copy_messages(&self, src: &str, dst: &str, uids: &[String]) -> Result<()> {
            let client = self.inner.clone();
            let src = src.to_string();
            let dst = dst.to_string();
            let uids = uids.to_vec();
            tokio::task::spawn_blocking(move || {
                client.copy_messages(&src, &dst, &uids)
                    .map_err(|e| EmailError::Backend(e.to_string()))
            }).await?
        }

        async fn move_messages(&self, src: &str, dst: &str, uids: &[String]) -> Result<()> {
            let client = self.inner.clone();
            let src = src.to_string();
            let dst = dst.to_string();
            let uids = uids.to_vec();
            tokio::task::spawn_blocking(move || {
                client.move_messages(&src, &dst, &uids)
                    .map_err(|e| EmailError::Backend(e.to_string()))
            }).await?
        }

        async fn delete_messages(&self, mailbox: &str, uids: &[String]) -> Result<()> {
            let client = self.inner.clone();
            let mailbox = mailbox.to_string();
            let uids = uids.to_vec();
            tokio::task::spawn_blocking(move || {
                client.delete_messages(&mailbox, &uids)
                    .map_err(|e| EmailError::Backend(e.to_string()))
            }).await?
        }

        async fn store_flags(&self, mailbox: &str, uids: &[String], flags: &[Flag], add: bool) -> Result<()> {
            tokio::task::spawn_blocking(move || Ok(())).await?
        }

        async fn send_message(&self, message: &EmailMessage) -> Result<()> {
            Err(EmailError::InvalidOperation("Maildir backend does not support sending".to_string()))
        }
    }
}

/// Factory function to build client from config
pub async fn build_client(config: &crate::config::EmailConfig) -> Result<Arc<dyn EmailClient>> {
    match &config.backend {
        #[cfg(feature = "imap")]
        crate::config::EmailBackend::Imap { server, username } => {
            let password = std::env::var(format!("HACIENDA_EMAIL_{}_PASSWORD", config.account.to_uppercase()))
                .map_err(|_| EmailError::Auth("IMAP password not found in environment".to_string()))?;
            let client = imap_client::IoEmailImapClient::new(server, username, secrecy::SecretString::new(password.into()))?;
            Ok(Arc::new(client))
        }
        #[cfg(feature = "maildir")]
        crate::config::EmailBackend::Maildir { path } => {
            let client = maildir_client::IoEmailMaildirClient::new(path)?;
            Ok(Arc::new(client))
        }
        _ => Err(EmailError::Config("Backend not enabled or not implemented".to_string())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_flag_serialization() {
        let flag = Flag::Seen;
        let json = serde_json::to_string(&flag).unwrap();
        assert_eq!(json, ""Seen"");
    }
}
