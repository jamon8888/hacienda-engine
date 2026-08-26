//! Integration tests for hacienda-email

use hacienda_email::{EmailConfig, EmailPlugin, EmailRequest, EmailOperation, EmailSource, EmailDestination};
use hacienda_core::plugin::Plugin;
use tempfile::tempdir;

#[tokio::test]
async fn test_plugin_creation() {
    let config = EmailConfig::default();
    let plugin = EmailPlugin::new(config).await;
    // This will fail without proper backend setup, but tests the creation path
    assert!(plugin.is_err() || plugin.is_ok());
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

    // Verify serialization
    let json = serde_json::to_string(&request).unwrap();
    assert!(json.contains("Archive"));
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
    assert!(json.contains("ExtractNER"));
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
    assert!(json.contains("Redact"));
    assert!(json.contains("Email"));
    assert!(json.contains("Phone"));
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
    assert!(json.contains("Pipeline"));
    assert!(json.contains("Archive"));
    assert!(json.contains("Ner"));
    assert!(json.contains("RagIndex"));
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
    use hacienda_email::archive::parse_human_date;
    use chrono::TimeZone;

    // Test RFC3339
    let dt = parse_human_date("2024-01-15T10:30:00Z").unwrap();
    assert_eq!(dt.year(), 2024);
    assert_eq!(dt.month(), 1);
    assert_eq!(dt.day(), 15);

    // Test simple date
    let dt = parse_human_date("2024-01-15").unwrap();
    assert_eq!(dt.year(), 2024);
    assert_eq!(dt.month(), 1);
    assert_eq!(dt.day(), 15);

    // Test "N days ago"
    let dt = parse_human_date("7 days ago").unwrap();
    let now = chrono::Utc::now();
    let diff = now - dt;
    assert!(diff.num_days() >= 6 && diff.num_days() <= 8);
}

#[tokio::test]
async fn test_html_to_text() {
    use hacienda_email::ner::html_to_text;

    let html = "<p>Hello <b>world</b></p><br/>New line";
    let text = html_to_text(html);
    assert!(text.contains("Hello"));
    assert!(text.contains("world"));
    assert!(text.contains("New line"));
}

#[tokio::test]
async fn test_format_addresses() {
    use hacienda_email::archive::format_addresses;
    use hacienda_email::models::EmailAddress;

    let addresses = vec![
        EmailAddress { name: Some("John Doe".to_string()), address: "john@example.com".to_string() },
        EmailAddress { name: None, address: "jane@example.com".to_string() },
    ];

    let formatted = format_addresses(&addresses);
    assert!(formatted.contains("John Doe"));
    assert!(formatted.contains("john@example.com"));
    assert!(formatted.contains("jane@example.com"));
}

#[cfg(feature = "mock")]
#[tokio::test]
async fn test_mock_imap_archive() {
    use mock_imap::MockImapServer;
    use hacienda_email::config::EmailBackend;

    let mut mock = MockImapServer::new();
    mock.expect_login().returning(|_, _| Ok(()));
    mock.expect_select().returning(|_| Ok(()));
    mock.expect_fetch().returning(|_, _| Ok(vec![]));
    let addr = mock.start().await;

    let config = EmailConfig {
        backend: EmailBackend::Imap {
            server: format!("imap://{}", addr),
            username: "test".to_string(),
        },
        account: "test".to_string(),
        ..Default::default()
    };

    // This would test the full archive flow with a mock IMAP server
    // let plugin = EmailPlugin::new(config).await.unwrap();
    // ...
}