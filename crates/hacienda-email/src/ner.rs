use crate::backend::EmailClient;
use crate::config::EmailConfig;
use crate::error::{EmailError, Result};
use crate::models::{EmailOperation, EmailSource, ExtractedEntity};
use crate::pii::PiiEngine;
use std::path::Path;

/// Extract named entities from emails
pub async fn extract_ner(
    client: &dyn EmailClient,
    pii: &PiiEngine,
    op: EmailOperation,
    config: &EmailConfig,
) -> Result<crate::models::EmailResponse> {
    let EmailOperation::ExtractNER {
        source,
        output,
        model,
    } = op
    else {
        return Err(EmailError::InvalidOperation(
            "Expected ExtractNER operation".to_string(),
        ));
    };

    let envelopes = fetch_envelopes(client, &source).await?;
    // The model name is accepted for API compatibility; hacienda-core's shared
    // detection pipeline owns the configured NER backend.
    let _model_name = model.unwrap_or_else(|| config.ner.model.clone());

    let mut all_entities = Vec::new();
    let mut errors = Vec::new();

    // Process in batches
    for chunk in envelopes.chunks(config.ner.batch_size) {
        for envelope in chunk {
            match client.get_message("INBOX", &envelope.id).await {
                Ok(message) => {
                    let text = extract_text(&message);
                    match extract_entities(&text, pii).await {
                        Ok(entities) => {
                            for entity in entities {
                                all_entities.push(ExtractedEntity {
                                    text: entity.text,
                                    label: entity.label,
                                    confidence: entity.confidence,
                                    email_id: envelope.id.clone(),
                                    span: entity.span,
                                });
                            }
                        }
                        Err(e) => errors.push(format!("NER failed for {}: {}", envelope.id, e)),
                    }
                }
                Err(e) => errors.push(format!("Failed to fetch {}: {}", envelope.id, e)),
            }
        }
    }

    // Save entities to JSON. Each error names the operation and output path so
    // the caller can tell a directory failure from a write failure.
    if let Some(parent) = Path::new(&output).parent() {
        tokio::fs::create_dir_all(parent).await.map_err(|e| {
            EmailError::Io(std::io::Error::other(format!(
                "NER: failed to create output directory '{}': {e}. \
                 Check the path is writable and the parent exists.",
                parent.display()
            )))
        })?;
    }
    let json = serde_json::to_string_pretty(&all_entities).map_err(|e| {
        EmailError::Ner(format!(
            "NER: failed to serialize {} entities for '{output}': {e}. \
             Retry after checking the result data is serializable.",
            all_entities.len()
        ))
    })?;
    tokio::fs::write(&output, json).await.map_err(|e| {
        EmailError::Io(std::io::Error::other(format!(
            "NER: failed to write entities to '{output}': {e}. \
             Check the path is writable and free disk space is available."
        )))
    })?;

    Ok(crate::models::EmailResponse {
        success: errors.is_empty(),
        processed_count: all_entities.len(),
        output_path: Some(output),
        entities: Some(all_entities),
        errors,
        ..Default::default()
    })
}

/// Fetch envelopes for NER processing
async fn fetch_envelopes(
    client: &dyn EmailClient,
    source: &EmailSource,
) -> Result<Vec<crate::models::Envelope>> {
    match source {
        EmailSource::Account { name: _, mailbox } => {
            let mb = mailbox.as_deref().unwrap_or("INBOX");
            client.list_envelopes(mb, Some(1000)).await
        }
        EmailSource::Maildir { path: _ } => client.list_envelopes("INBOX", Some(1000)).await,
        EmailSource::Files { paths: _ } => Err(EmailError::InvalidOperation(
            "File source not yet implemented".to_string(),
        )),
        EmailSource::Stdin => Err(EmailError::InvalidOperation(
            "Stdin source not yet implemented".to_string(),
        )),
    }
}

/// Extract text content from email message
fn extract_text(message: &crate::models::EmailMessage) -> String {
    let mut parts = Vec::new();

    if !message.subject.is_empty() {
        parts.push(format!("Subject: {}", message.subject));
    }

    if let Some(text) = &message.text_body {
        parts.push(text.clone());
    } else if let Some(html) = &message.html_body {
        // Simple HTML to text conversion
        parts.push(html_to_text(html));
    }

    parts.join("\n\n")
}

/// Simple HTML to text conversion
pub fn html_to_text(html: &str) -> String {
    // Very basic - in production use a proper HTML parser
    html.replace("<br>", "\n")
        .replace("<br/>", "\n")
        .replace("<br />", "\n")
        .replace("<p>", "\n\n")
        .replace("</p>", "")
        .replace("<div>", "\n")
        .replace("</div>", "")
        .replace("&nbsp;", " ")
        .replace("&amp;", "&")
        // Strip remaining tags
        .split('<')
        .map(|s| s.split('>').next_back().unwrap_or(s))
        .collect::<Vec<_>>()
        .join("")
}

/// Internal entity representation from NER
#[derive(Debug, Clone)]
struct NerEntity {
    text: String,
    label: String,
    confidence: f32,
    span: (usize, usize),
}

/// Extract entities via hacienda-core's real detection pipeline.
async fn extract_entities(text: &str, pii: &PiiEngine) -> Result<Vec<NerEntity>> {
    let found = pii.scan_entities(text).await?;
    Ok(found
        .into_iter()
        .map(|(label, mention, confidence, span)| NerEntity {
            text: mention,
            label,
            confidence,
            span,
        })
        .collect())
}
