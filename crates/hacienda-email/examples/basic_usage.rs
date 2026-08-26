//! Basic usage example for hacienda-email
//!
//! This example demonstrates how to use the email processor plugin
//! to archive, extract NER, index for RAG, and redact emails.

use hacienda_email::{EmailConfig, EmailPlugin, EmailRequest, EmailOperation, EmailSource, EmailDestination, PipelineStep};
use hacienda_core::plugin::Plugin;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Load configuration
    let config = EmailConfig {
        backend: hacienda_email::config::EmailBackend::Imap {
            server: "imaps://imap.gmail.com".to_string(),
            username: "user@gmail.com".to_string(),
        },
        account: "work".to_string(),
        archive_path: Some("./email-archive".to_string()),
        ..Default::default()
    };

    // Create plugin instance
    let plugin = EmailPlugin::new(config).await?;
    println!("Plugin initialized: {}", plugin.metadata().name);

    // Example 1: Archive emails
    println!("
=== Archiving emails ===");
    let archive_request = EmailRequest {
        operation: EmailOperation::Archive {
            source: EmailSource::Account {
                name: "work".to_string(),
                mailbox: Some("INBOX".to_string()),
            },
            destination: EmailDestination::Maildir {
                path: "./email-archive".to_string(),
            },
            since: Some("30 days ago".to_string()),
            before: None,
            mailbox: Some("INBOX".to_string()),
            include_attachments: true,
        },
        config_overrides: None,
    };

    let response = plugin.process(archive_request).await?;
    println!("Archived {} emails", response.processed_count);
    if let Some(path) = response.output_path {
        println!("Saved to: {}", path);
    }
    for err in &response.errors {
        eprintln!("Error: {}", err);
    }

    // Example 2: Extract NER
    println!("
=== Extracting Named Entities ===");
    let ner_request = EmailRequest {
        operation: EmailOperation::ExtractNER {
            source: EmailSource::Maildir {
                path: "./email-archive".to_string(),
            },
            output: "./entities.json".to_string(),
            model: Some("gliner-multi-v2.1".to_string()),
        },
        config_overrides: None,
    };

    let response = plugin.process(ner_request).await?;
    println!("Extracted {} entities", response.processed_count);
    if let Some(entities) = response.entities {
        for entity in entities.iter().take(5) {
            println!("  {}: {} (confidence: {:.2})", entity.label, entity.text, entity.confidence);
        }
    }

    // Example 3: Index for RAG
    println!("
=== Indexing for RAG ===");
    let rag_request = EmailRequest {
        operation: EmailOperation::IndexForRag {
            source: EmailSource::Maildir {
                path: "./email-archive".to_string(),
            },
            index_name: Some("email-corpus".to_string()),
            chunk_size: Some(512),
        },
        config_overrides: None,
    };

    let response = plugin.process(rag_request).await?;
    println!("Created {} chunks", response.processed_count);
    if let Some(stats) = response.rag_stats {
        println!("Indexed {} vectors in {}", stats.vectors_indexed, stats.index_name);
    }

    // Example 4: Redact PII
    println!("
=== Redacting PII ===");
    let redact_request = EmailRequest {
        operation: EmailOperation::Redact {
            source: EmailSource::Maildir {
                path: "./email-archive".to_string(),
            },
            destination: EmailDestination::Maildir {
                path: "./email-redacted".to_string(),
            },
            rules: None, // Use defaults from config
        },
        config_overrides: None,
    };

    let response = plugin.process(redact_request).await?;
    println!("Redacted {} emails", response.processed_count);
    if let Some(stats) = response.redaction_stats {
        println!("Total redactions: {}", stats.total_redactions);
        for (rule, count) in stats.by_type {
            println!("  {}: {}", rule, count);
        }
    }

    // Example 5: Full pipeline
    println!("
=== Running Full Pipeline ===");
    let pipeline_request = EmailRequest {
        operation: EmailOperation::Pipeline {
            source: EmailSource::Account {
                name: "work".to_string(),
                mailbox: Some("INBOX".to_string()),
            },
            destination: EmailDestination::Maildir {
                path: "./email-processed".to_string(),
            },
            steps: vec![
                PipelineStep::Archive,
                PipelineStep::ExtractAttachments,
                PipelineStep::Ner,
                PipelineStep::RagIndex,
                PipelineStep::Redact,
            ],
        },
        config_overrides: None,
    };

    let response = plugin.process(pipeline_request).await?;
    println!("Pipeline completed: {} emails processed", response.processed_count);

    Ok(())
}