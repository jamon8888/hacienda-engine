//! Basic usage example for hacienda-email.
//!
//! Demonstrates the `EmailProcessor` API for archiving, extracting NER,
//! indexing for RAG and redacting PII. Uses a local Maildir backend so the
//! example runs without a live mail server (point `path` at a real Maildir to
//! process actual messages).

use hacienda_email::{
    EmailConfig, EmailDestination, EmailOperation, EmailProcessor, EmailRequest, EmailSource,
    PipelineStep,
};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config = EmailConfig {
        backend: hacienda_email::config::EmailBackend::Maildir {
            path: "./email-archive".to_string(),
        },
        account: "work".to_string(),
        archive_path: Some("./email-archive".to_string()),
        ..Default::default()
    };

    let processor = EmailProcessor::new(config).await?;
    println!("EmailProcessor initialized");

    // Example 1: Archive emails from the INBOX into a local Maildir.
    println!("\n=== Archiving emails ===");
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

    let response = processor.process(archive_request).await?;
    println!("Archived {} emails", response.processed_count);
    for err in &response.errors {
        eprintln!("Error: {err}");
    }

    // Example 2: Extract named entities.
    println!("\n=== Extracting Named Entities ===");
    let ner_request = EmailRequest {
        operation: EmailOperation::ExtractNER {
            source: EmailSource::Maildir {
                path: "./email-archive".to_string(),
            },
            output: "./entities.json".to_string(),
            model: None,
        },
        config_overrides: None,
    };

    let response = processor.process(ner_request).await?;
    println!("Extracted {} entities", response.processed_count);
    if let Some(entities) = response.entities {
        for entity in entities.iter().take(5) {
            println!(
                "  {}: {} (confidence: {:.2})",
                entity.label, entity.text, entity.confidence
            );
        }
    }

    // Example 3: Index for RAG.
    println!("\n=== Indexing for RAG ===");
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

    let response = processor.process(rag_request).await?;
    println!("Created {} chunks", response.processed_count);
    if let Some(stats) = response.rag_stats {
        println!("Indexed {} vectors in {}", stats.vectors_indexed, stats.index_name);
    }

    // Example 4: Redact PII.
    println!("\n=== Redacting PII ===");
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

    let response = processor.process(redact_request).await?;
    println!("Redacted {} emails", response.processed_count);
    if let Some(stats) = response.redaction_stats {
        println!("Total redactions: {}", stats.total_redactions);
        for (rule, count) in stats.by_type {
            println!("  {rule}: {count}");
        }
    }

    // Example 5: Full pipeline.
    println!("\n=== Running Full Pipeline ===");
    let pipeline_request = EmailRequest {
        operation: EmailOperation::Pipeline {
            source: EmailSource::Maildir {
                path: "./email-archive".to_string(),
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

    let response = processor.process(pipeline_request).await?;
    println!("Pipeline completed: {} emails processed", response.processed_count);

    Ok(())
}
