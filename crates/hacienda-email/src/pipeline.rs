use crate::backend::EmailClient;
use crate::config::EmailConfig;
use crate::error::{EmailError, Result};
use crate::models::{EmailDestination, EmailOperation, EmailSource, PipelineStep};

/// Run full email processing pipeline
pub async fn run_pipeline(
    client: &dyn EmailClient,
    op: EmailOperation,
    config: &EmailConfig,
) -> Result<crate::models::EmailResponse> {
    let EmailOperation::Pipeline {
        source,
        destination,
        steps,
    } = op else {
        return Err(EmailError::InvalidOperation("Expected Pipeline operation".to_string()));
    };

    let mut current_source = source;
    let mut total_processed = 0;
    let mut all_entities = Vec::new();
    let mut total_chunks = 0;
    let mut total_redactions = 0;
    let mut all_errors = Vec::new();

    for step in steps {
        match step {
            PipelineStep::Archive => {
                let op = EmailOperation::Archive {
                    source: current_source.clone(),
                    destination: destination.clone(),
                    since: None,
                    before: None,
                    mailbox: None,
                    include_attachments: true,
                };
                let response = crate::archive::archive(client, op, config).await?;
                total_processed = response.processed_count;
                all_errors.extend(response.errors);

                // Update source for next step
                if let EmailDestination::Maildir { path } = &destination {
                    current_source = EmailSource::Maildir { path: path.clone() };
                } else if let EmailDestination::Files { path } = &destination {
                    current_source = EmailSource::Files { paths: vec![path.clone()] };
                }
            }

            PipelineStep::ExtractAttachments => {
                // Extract attachments from archived emails
                let extracted = extract_attachments_from_source(client, &current_source).await?;
                all_errors.extend(extracted.errors);
                total_processed += extracted.count;
            }

            PipelineStep::Ner => {
                let op = EmailOperation::ExtractNER {
                    source: current_source.clone(),
                    output: format!("{}/entities.json", destination_path(&destination)),
                    model: None,
                };
                let response = crate::ner::extract_ner(client, op, config).await?;
                all_entities.extend(response.entities.unwrap_or_default());
                all_errors.extend(response.errors);
                total_processed = response.processed_count;
            }

            PipelineStep::RagIndex => {
                let op = EmailOperation::IndexForRag {
                    source: current_source.clone(),
                    index_name: None,
                    chunk_size: None,
                };
                let response = crate::rag::index_for_rag(client, op, config).await?;
                if let Some(stats) = response.rag_stats {
                    total_chunks = stats.chunks_created;
                }
                all_errors.extend(response.errors);
                total_processed = response.processed_count;
            }

            PipelineStep::Redact => {
                let op = EmailOperation::Redact {
                    source: current_source.clone(),
                    destination: destination.clone(),
                    rules: None,
                };
                let response = crate::redact::redact(client, op, config).await?;
                if let Some(stats) = response.redaction_stats {
                    total_redactions = stats.total_redactions;
                }
                all_errors.extend(response.errors);
                total_processed = response.processed_count;
            }
        }
    }

    Ok(crate::models::EmailResponse {
        success: all_errors.is_empty(),
        processed_count: total_processed,
        output_path: Some(destination_path(&destination)),
        entities: if all_entities.is_empty() { None } else { Some(all_entities) },
        rag_stats: Some(crate::models::RagIndexStats {
            chunks_created: total_chunks,
            vectors_indexed: total_chunks,
            index_name: config.rag.index_name.clone(),
        }),
        redaction_stats: Some(crate::models::RedactionStats {
            total_redactions,
            by_type: std::collections::HashMap::new(),
        }),
        errors: all_errors,
    })
}

/// Extract path from destination
fn destination_path(dest: &EmailDestination) -> String {
    match dest {
        EmailDestination::Maildir { path } | EmailDestination::Files { path } => path.clone(),
        EmailDestination::Stdout => "stdout".to_string(),
    }
}

/// Extract attachments from source
async fn extract_attachments_from_source(
    client: &dyn EmailClient,
    source: &EmailSource,
) -> Result<AttachmentResult> {
    let envelopes = match source {
        EmailSource::Account { name: _, mailbox } => {
            let mb = mailbox.as_deref().unwrap_or("INBOX");
            client.list_envelopes(mb, Some(1000)).await?
        }
        EmailSource::Maildir { path: _ } => {
            client.list_envelopes("INBOX", Some(1000)).await?
        }
        _ => return Ok(AttachmentResult { count: 0, errors: Vec::new() }),
    };

    let mut count = 0;
    let mut errors = Vec::new();

    for envelope in envelopes {
        match client.get_message("INBOX", &envelope.id).await {
            Ok(message) => {
                if !message.attachments.is_empty() {
                    count += message.attachments.len();
                    // In real implementation, extract and save attachments
                }
            }
            Err(e) => errors.push(format!("Failed to fetch {}: {}", envelope.id, e)),
        }
    }

    Ok(AttachmentResult { count, errors })
}

/// Result of attachment extraction
struct AttachmentResult {
    count: usize,
    errors: Vec<String>,
}
