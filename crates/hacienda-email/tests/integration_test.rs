//! Integration tests for hacienda-email.
//!
//! These exercise the public request/response API without requiring a live
//! mail server: `EmailProcessor::new` with a Maildir backend only roots a
//! local client (no network), and the operation tests verify the serde shape
//! that the CLI / HTTP / MCP surfaces will depend on.

use hacienda_email::{EmailConfig, EmailOperation, EmailRequest, EmailSource, EmailDestination};
use tempfile::tempdir;

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
    let processor = hacienda_email::EmailProcessor::new(config).await;
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
    assert!(json.contains("\"operation\":\"archive\""), "JSON was: {json}");
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
    assert!(json.contains("\"operation\":\"extract_ner\""), "JSON was: {json}");
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
    assert!(json.contains("\"operation\":\"redact\""), "JSON was: {json}");
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
    assert!(json.contains("\"operation\":\"pipeline\""), "JSON was: {json}");
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
async fn test_format_addresses() {
    use hacienda_email::models::EmailAddress;

    let addresses = vec![
        EmailAddress { name: Some("John Doe".to_string()), address: "john@example.com".to_string() },
        EmailAddress { name: None, address: "jane@example.com".to_string() },
    ];

    let formatted = hacienda_email::archive::format_addresses(&addresses);
    assert!(formatted.contains("John Doe"));
    assert!(formatted.contains("john@example.com"));
    assert!(formatted.contains("jane@example.com"));
}
