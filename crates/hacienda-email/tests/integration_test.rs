//! Integration tests for hacienda-email.
//!
//! These exercise the public request/response API without requiring a live
//! mail server: `EmailProcessor::new` with a Maildir backend only roots a
//! local client (no network), and the operation tests verify the serde shape
//! that the CLI / HTTP / MCP surfaces will depend on.

use hacienda_core::auth::{AuthContext, Caller, Capability, CapabilitySet};
use hacienda_email::{
    pii::PiiEngine, EmailConfig, EmailDestination, EmailOperation, EmailRequest, EmailSource,
};
use tempfile::tempdir;

fn test_pii() -> PiiEngine {
    PiiEngine::for_local_testing()
}

fn test_ctx() -> AuthContext {
    AuthContext::with_tenant(
        "test",
        hacienda_core::tenancy::TenantId::default_tenant(),
        CapabilitySet::new([Capability::DocumentsProcess]),
    )
}

/// In-memory fixture implementing the backend trait so the real redaction path
/// can run offline and its on-disk RFC822 output can be inspected.
struct FixtureClient {
    envelope: hacienda_email::models::Envelope,
    message: hacienda_email::models::EmailMessage,
}

#[async_trait::async_trait]
impl hacienda_email::backend::EmailClient for FixtureClient {
    async fn list_mailboxes(&self) -> hacienda_email::Result<Vec<hacienda_email::models::Mailbox>> {
        Ok(Vec::new())
    }
    async fn list_envelopes(
        &self,
        _mailbox: &str,
        _limit: Option<usize>,
    ) -> hacienda_email::Result<Vec<hacienda_email::models::Envelope>> {
        Ok(vec![self.envelope.clone()])
    }
    async fn search_envelopes(
        &self,
        _mailbox: &str,
        _query: &str,
    ) -> hacienda_email::Result<Vec<hacienda_email::models::Envelope>> {
        Ok(Vec::new())
    }
    async fn get_message(
        &self,
        _mailbox: &str,
        _uid: &str,
    ) -> hacienda_email::Result<hacienda_email::models::EmailMessage> {
        Ok(self.message.clone())
    }
    async fn add_message(
        &self,
        _mailbox: &str,
        _message: &hacienda_email::models::EmailMessage,
    ) -> hacienda_email::Result<()> {
        Ok(())
    }
    async fn copy_messages(
        &self,
        _src: &str,
        _dst: &str,
        _uids: &[String],
    ) -> hacienda_email::Result<()> {
        Ok(())
    }
    async fn move_messages(
        &self,
        _src: &str,
        _dst: &str,
        _uids: &[String],
    ) -> hacienda_email::Result<()> {
        Ok(())
    }
    async fn delete_messages(
        &self,
        _mailbox: &str,
        _uids: &[String],
    ) -> hacienda_email::Result<()> {
        Ok(())
    }
    async fn store_flags(
        &self,
        _mailbox: &str,
        _uids: &[String],
        _flags: &[hacienda_email::models::Flag],
        _add: bool,
    ) -> hacienda_email::Result<()> {
        Ok(())
    }
    async fn send_message(
        &self,
        _message: &hacienda_email::models::EmailMessage,
    ) -> hacienda_email::Result<()> {
        Ok(())
    }
}

#[tokio::test]
async fn processor_builds_with_maildir_backend() {
    let dir = tempdir().unwrap();
    let config = EmailConfig {
        backend: hacienda_email::config::EmailBackend::Maildir {
            path: dir.path().to_str().unwrap().to_string(),
        },
        account: "test".to_string(),
        ..Default::default()
    };
    // Construction roots a local Maildir client; no I/O happens yet.
    let processor = hacienda_email::EmailProcessor::new(config, test_pii()).await;
    assert!(processor.is_ok(), "Maildir processor should build");
}

#[tokio::test]
async fn test_archive_operation_structure() {
    let request = EmailRequest {
        operation: EmailOperation::Archive {
            source: EmailSource::Account {
                name: "test".to_string(),
                mailbox: Some("INBOX".to_string()),
            },
            destination: EmailDestination::Maildir {
                path: "/tmp/test".to_string(),
            },
            since: Some("7 days ago".to_string()),
            before: None,
            mailbox: Some("INBOX".to_string()),
            include_attachments: true,
        },
        config_overrides: None,
    };

    let json = serde_json::to_string(&request).unwrap();
    assert!(
        json.contains("\"operation\":\"archive\""),
        "JSON was: {json}"
    );
    assert!(json.contains("test"));
}

#[tokio::test]
async fn test_ner_operation_structure() {
    let request = EmailRequest {
        operation: EmailOperation::ExtractNER {
            source: EmailSource::Files {
                paths: vec!["/tmp/test.eml".to_string()],
            },
            output: "/tmp/entities.json".to_string(),
            model: Some("gliner-multi-v2.1".to_string()),
        },
        config_overrides: None,
    };

    let json = serde_json::to_string(&request).unwrap();
    assert!(
        json.contains("\"operation\":\"extract_n_e_r\""),
        "JSON was: {json}"
    );
    assert!(json.contains("gliner-multi-v2.1"));
}

#[tokio::test]
async fn test_redact_operation_structure() {
    let request = EmailRequest {
        operation: EmailOperation::Redact {
            source: EmailSource::Maildir {
                path: "/tmp/mail".to_string(),
            },
            destination: EmailDestination::Files {
                path: "/tmp/redacted".to_string(),
            },
            rules: Some(vec![
                hacienda_email::config::RedactionRule::Email,
                hacienda_email::config::RedactionRule::Phone,
            ]),
        },
        config_overrides: None,
    };

    let json = serde_json::to_string(&request).unwrap();
    assert!(
        json.contains("\"operation\":\"redact\""),
        "JSON was: {json}"
    );
    assert!(json.contains("email"));
    assert!(json.contains("phone"));
}

#[tokio::test]
async fn test_pipeline_operation_structure() {
    let request = EmailRequest {
        operation: EmailOperation::Pipeline {
            source: EmailSource::Account {
                name: "test".to_string(),
                mailbox: None,
            },
            destination: EmailDestination::Maildir {
                path: "/tmp/pipeline".to_string(),
            },
            steps: vec![
                hacienda_email::models::PipelineStep::Archive,
                hacienda_email::models::PipelineStep::Ner,
                hacienda_email::models::PipelineStep::RagIndex,
            ],
        },
        config_overrides: None,
    };

    let json = serde_json::to_string(&request).unwrap();
    assert!(
        json.contains("\"operation\":\"pipeline\""),
        "JSON was: {json}"
    );
    assert!(json.contains("\"archive\""));
    assert!(json.contains("\"ner\""));
    assert!(json.contains("\"rag_index\""));
}

#[tokio::test]
async fn test_config_defaults() {
    let config = EmailConfig::default();
    assert_eq!(config.account, "default");
    assert_eq!(config.ner.confidence_threshold, 0.5);
    assert_eq!(config.rag.chunk_size, 512);
    assert_eq!(config.redaction.replacement, "[REDACTED]");
}

#[tokio::test]
async fn test_config_serialization() {
    let config = EmailConfig::default();
    let toml = toml::to_string(&config).unwrap();
    assert!(toml.contains("backend"));
    assert!(toml.contains("ner"));
    assert!(toml.contains("rag"));
    assert!(toml.contains("redaction"));
}

#[tokio::test]
async fn test_date_parsing() {
    use chrono::Datelike;

    let dt = hacienda_email::archive::parse_human_date("2024-01-15T10:30:00Z").unwrap();
    assert_eq!(dt.year(), 2024);
    assert_eq!(dt.month(), 1);
    assert_eq!(dt.day(), 15);

    let dt = hacienda_email::archive::parse_human_date("2024-01-15").unwrap();
    assert_eq!(dt.year(), 2024);
    assert_eq!(dt.month(), 1);
    assert_eq!(dt.day(), 15);

    let dt = hacienda_email::archive::parse_human_date("7 days ago").unwrap();
    let now = chrono::Utc::now();
    let diff = now - dt;
    assert!(diff.num_days() >= 6 && diff.num_days() <= 8);
}

#[tokio::test]
async fn test_html_to_text() {
    let html = "<p>Hello <b>world</b></p><br/>New line";
    let text = hacienda_email::ner::html_to_text(html);
    assert!(text.contains("Hello"));
    assert!(text.contains("world"));
    assert!(text.contains("New line"));
}

#[tokio::test]
async fn test_oauth_provider_defaults() {
    use hacienda_email::OAuthProvider;
    assert!(OAuthProvider::Gmail.imap_url().contains("imap.gmail.com"));
    assert!(OAuthProvider::Outlook
        .imap_url()
        .contains("outlook.office365.com"));

    // An OAuth backend must round-trip through serde for config files.
    let cfg = EmailConfig {
        backend: hacienda_email::config::EmailBackend::OAuth {
            server: String::new(),
            username: "user@example.com".to_string(),
            provider: OAuthProvider::Gmail,
            token_command: None,
        },
        ..Default::default()
    };
    let toml = toml::to_string(&cfg).unwrap();
    assert!(toml.contains("oauth"));
    assert!(toml.contains("gmail"));
}

#[tokio::test]
async fn test_privacy_mode_defaults_to_redact() {
    assert_eq!(
        EmailConfig::default().redaction.mode,
        hacienda_email::PrivacyMode::Redact
    );
}

#[tokio::test]
async fn test_format_addresses() {
    use hacienda_email::models::EmailAddress;

    let addresses = vec![
        EmailAddress {
            name: Some("John Doe".to_string()),
            address: "john@example.com".to_string(),
        },
        EmailAddress {
            name: None,
            address: "jane@example.com".to_string(),
        },
    ];

    let formatted = hacienda_email::archive::format_addresses(&addresses);
    assert!(formatted.contains("John Doe"));
    assert!(formatted.contains("john@example.com"));
    assert!(formatted.contains("jane@example.com"));
}

/// P0.1 non-egress test through the real ingestion path: the clear subject and
/// display names must be absent from every persisted RAG surface (document
/// summary, full record, chunks and a full snapshot).
#[tokio::test]
async fn rag_ingestion_never_persists_clear_subject_or_display_names() {
    use hacienda_email::models::EmailAddress;
    use hacienda_rag::{CollectionSpec, InMemoryVectorStore, RagStore};

    let envelope = hacienda_email::models::Envelope {
        id: "uid-7".to_string(),
        mailbox: "INBOX".to_string(),
        subject: "Jane Doe SSN 123-45-6789 private review".to_string(),
        from: vec![EmailAddress {
            name: Some("Jane Doe".to_string()),
            address: "jane.doe@acme.example".to_string(),
        }],
        to: vec![EmailAddress {
            name: Some("Richard Roe".to_string()),
            address: "richard.roe@acme.example".to_string(),
        }],
        date: chrono::Utc::now(),
        flags: Vec::new(),
        has_attachments: false,
    };
    let message = hacienda_email::models::EmailMessage {
        id: "uid-7".to_string(),
        mailbox: "INBOX".to_string(),
        subject: envelope.subject.clone(),
        from: envelope.from.clone(),
        to: envelope.to.clone(),
        cc: Vec::new(),
        bcc: Vec::new(),
        date: chrono::Utc::now(),
        text_body: Some("Contact jane.doe@acme.example for details".to_string()),
        html_body: None,
        attachments: Vec::new(),
        flags: Vec::new(),
    };
    let client = FixtureClient { envelope, message };
    let config = EmailConfig::default();
    let pii = test_pii();
    let ctx = test_ctx();
    let caller = Caller::Principal(&ctx);

    let store = InMemoryVectorStore::new("email-rag-e2e");
    let collection = "email-corpus";
    store
        .ensure_collection(&CollectionSpec::new(collection, 0))
        .await
        .unwrap();
    let rag = hacienda_email::rag::RagIngestion {
        store: &store,
        dossier_id: None,
        account: "work",
        tenant_id: Some("work"),
        page_size: 10,
        max_pages: 1,
        concurrency: 1,
        incremental: false,
        optimize_after_run: false,
        persist_watermark: false,
        on_page: None,
    };
    let op = EmailOperation::IndexForRag {
        source: EmailSource::Account {
            name: "test".to_string(),
            mailbox: None,
        },
        index_name: Some(collection.to_string()),
        chunk_size: Some(512),
    };

    let response =
        hacienda_email::rag::index_for_rag(&client, &pii, caller, op, &config, Some(rag))
            .await
            .unwrap();
    assert!(response.success, "errors: {:?}", response.errors);

    let (summaries, _) = store.list_documents(collection, 10, 0).await.unwrap();
    assert_eq!(summaries.len(), 1);
    let (record, chunks) = store
        .get_document_chunks(collection, &summaries[0].id)
        .await
        .unwrap()
        .unwrap();
    let snapshot = serde_json::to_string(&(&summaries, &record, &chunks)).unwrap();
    for fixture in [
        "Jane Doe",
        "Richard Roe",
        "jane.doe@acme.example",
        "richard.roe@acme.example",
        "123-45-6789",
        "private review",
    ] {
        assert!(
            !snapshot.contains(fixture),
            "clear fixture leaked into RAG ingestion: {fixture}\nstore:\n{snapshot}"
        );
    }
}

/// P0.3 non-egress test through the real redact operation: the file written to
/// disk must contain no clear HTML, body PII, subject, display name, address or
/// attachment name from the fixture.
#[tokio::test]
async fn redact_operation_writes_content_free_rfc822() {
    use hacienda_email::models::{EmailAddress, EmailAttachment, EmailMessage};

    let dir = tempdir().unwrap();
    let envelope = hacienda_email::models::Envelope {
        id: "uid-1".to_string(),
        mailbox: "INBOX".to_string(),
        subject: "Jane Doe SSN 123-45-6789 review".to_string(),
        from: Vec::new(),
        to: Vec::new(),
        date: chrono::Utc::now(),
        flags: Vec::new(),
        has_attachments: true,
    };
    let message = EmailMessage {
        id: "uid-1".to_string(),
        mailbox: "INBOX".to_string(),
        subject: envelope.subject.clone(),
        from: vec![EmailAddress {
            name: Some("Jane Doe".to_string()),
            address: "jane.doe@acme.example".to_string(),
        }],
        to: vec![EmailAddress {
            name: Some("Richard Roe".to_string()),
            address: "richard.roe@acme.example".to_string(),
        }],
        cc: Vec::new(),
        bcc: Vec::new(),
        date: chrono::Utc::now(),
        text_body: Some("Call me at jane.doe@acme.example".to_string()),
        html_body: Some(
            "<html><body><p>Contact <b>jane.doe@acme.example</b></p>\
             <div data-secret=\"Jane Doe\">SSN 123-45-6789</div></body></html>"
                .to_string(),
        ),
        attachments: vec![EmailAttachment {
            filename: "jane-doe-contract.pdf".to_string(),
            content_type: "application/pdf".to_string(),
            size: 4,
            content_id: None,
            content: vec![0; 4],
        }],
        flags: Vec::new(),
    };
    let client = FixtureClient { envelope, message };
    let config = EmailConfig::default();
    let pii = test_pii();
    let ctx = test_ctx();
    let caller = Caller::Principal(&ctx);
    let op = EmailOperation::Redact {
        source: EmailSource::Account {
            name: "test".to_string(),
            mailbox: None,
        },
        destination: EmailDestination::Files {
            path: dir.path().to_str().unwrap().to_string(),
        },
        rules: None,
    };

    let response = hacienda_email::redact::redact(&client, &pii, caller, op, &config)
        .await
        .unwrap();
    assert!(response.success, "errors: {:?}", response.errors);
    assert_eq!(response.processed_count, 1);

    let written: Vec<_> = std::fs::read_dir(dir.path())
        .unwrap()
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "eml"))
        .collect();
    assert_eq!(written.len(), 1, "expected exactly one redacted .eml file");
    let rfc822 = std::fs::read_to_string(written[0].path()).unwrap();

    for fixture in [
        "Jane Doe",
        "Richard Roe",
        "jane.doe@acme.example",
        "richard.roe@acme.example",
        "123-45-6789",
        "jane-doe-contract.pdf",
        "<html",
        "<body",
        "data-secret",
        "<!--",
    ] {
        assert!(
            !rfc822.contains(fixture),
            "clear fixture leaked into redacted RFC822: {fixture}\nfile:\n{rfc822}"
        );
    }
}
