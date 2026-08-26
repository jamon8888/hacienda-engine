use crate::backend::EmailClient;
use crate::config::EmailConfig;
use crate::error::{EmailError, Result};
use crate::models::{EmailOperation, EmailSource, RagIndexStats};

/// Index emails for RAG
pub async fn index_for_rag(
    client: &dyn EmailClient,
    op: EmailOperation,
    config: &EmailConfig,
) -> Result<crate::models::EmailResponse> {
    let EmailOperation::IndexForRag {
        source,
        index_name,
        chunk_size,
    } = op else {
        return Err(EmailError::InvalidOperation("Expected IndexForRag operation".to_string()));
    };

    let envelopes = fetch_envelopes(client, &source).await?;
    let index = index_name.unwrap_or_else(|| config.rag.index_name.clone());
    let chunk_sz = chunk_size.unwrap_or(config.rag.chunk_size);
    let overlap = config.rag.chunk_overlap;

    let mut all_chunks = Vec::new();
    let mut errors = Vec::new();

    for envelope in envelopes {
        match client.get_message("INBOX", &envelope.id).await {
            Ok(message) => {
                let text = extract_text_for_rag(&message, &envelope);
                let chunks = chunk_text(&text, chunk_sz, overlap);
                for (i, chunk) in chunks.into_iter().enumerate() {
                    all_chunks.push(EmailChunk {
                        text: chunk,
                        email_id: envelope.id.clone(),
                        chunk_index: i,
                        subject: envelope.subject.clone(),
                        from: envelope.from.clone(),
                        date: envelope.date,
                    });
                }
            }
            Err(e) => errors.push(format!("Failed to fetch {}: {}", envelope.id, e)),
        }
    }

    // Index chunks via hacienda-rag
    let stats = index_chunks(&all_chunks, &index, &config.rag.embedding_model).await?;

    Ok(crate::models::EmailResponse {
        success: errors.is_empty(),
        processed_count: all_chunks.len(),
        rag_stats: Some(RagIndexStats {
            chunks_created: all_chunks.len(),
            vectors_indexed: stats.vectors_indexed,
            index_name: index,
        }),
        errors,
        ..Default::default()
    })
}

/// Fetch envelopes for RAG indexing
async fn fetch_envelopes(
    client: &dyn EmailClient,
    source: &EmailSource,
) -> Result<Vec<crate::models::Envelope>> {
    match source {
        EmailSource::Account { name: _, mailbox } => {
            let mb = mailbox.as_deref().unwrap_or("INBOX");
            client.list_envelopes(mb, Some(5000)).await
        }
        EmailSource::Maildir { path: _ } => {
            client.list_envelopes("INBOX", Some(5000)).await
        }
        EmailSource::Files { paths: _ } => {
            Err(EmailError::InvalidOperation("File source not yet implemented".to_string()))
        }
        EmailSource::Stdin => {
            Err(EmailError::InvalidOperation("Stdin source not yet implemented".to_string()))
        }
    }
}

/// Email chunk for RAG
#[derive(Debug, Clone)]
struct EmailChunk {
    text: String,
    email_id: String,
    chunk_index: usize,
    subject: String,
    from: Vec<crate::models::EmailAddress>,
    date: chrono::DateTime<chrono::Utc>,
}

/// Extract text for RAG with email metadata
fn extract_text_for_rag(
    message: &crate::models::EmailMessage,
    envelope: &crate::models::Envelope,
) -> String {
    let mut parts = Vec::new();

    parts.push(format!("Email ID: {}", envelope.id));
    parts.push(format!("Subject: {}", envelope.subject));
    parts.push(format!("From: {}", format_addresses(&envelope.from)));
    parts.push(format!("To: {}", format_addresses(&envelope.to)));
    parts.push(format!("Date: {}", envelope.date.format("%Y-%m-%d %H:%M:%S UTC")));

    if let Some(text) = &message.text_body {
        parts.push(format!("Content:\n{}", text));
    } else if let Some(html) = &message.html_body {
        parts.push(format!("Content:\n{}", html_to_text(html)));
    }

    parts.join("\n\n")
}

/// Format addresses
fn format_addresses(addresses: &[crate::models::EmailAddress]) -> String {
    addresses.iter()
        .map(|a| match &a.name {
            Some(name) => format!("{} <{}>", name, a.address),
            None => a.address.clone(),
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// HTML to text
fn html_to_text(html: &str) -> String {
    html.replace("<br>", "\n")
        .replace("<br/>", "\n")
        .replace("<p>", "\n\n")
        .replace("</p>", "")
        .split('<')
        .map(|s| s.split('>').last().unwrap_or(s))
        .collect::<Vec<_>>()
        .join("")
}

/// Simple text chunking
fn chunk_text(text: &str, chunk_size: usize, overlap: usize) -> Vec<String> {
    if text.len() <= chunk_size {
        return vec![text.to_string()];
    }

    let mut chunks = Vec::new();
    let mut start = 0;

    while start < text.len() {
        let end = std::cmp::min(start + chunk_size, text.len());
        chunks.push(text[start..end].to_string());

        if end == text.len() {
            break;
        }

        start = end.saturating_sub(overlap);
    }

    chunks
}

/// Index chunks using hacienda-rag
async fn index_chunks(
    chunks: &[EmailChunk],
    index_name: &str,
    embedding_model: &str,
) -> Result<IndexStats> {
    #[cfg(feature = "rag")]
    {
        use hacienda_rag::{RagPipeline, EmbeddingConfig};

        let embedding_config = EmbeddingConfig {
            model: embedding_model.to_string(),
            ..Default::default()
        };

        let pipeline = RagPipeline::new(embedding_config)
            .map_err(|e| EmailError::Rag(format!("Failed to create RAG pipeline: {}", e)))?;

        // Convert EmailChunk to hacienda_rag::Chunk
        let rag_chunks: Vec<_> = chunks.iter().map(|c| hacienda_rag::Chunk {
            text: c.text.clone(),
            metadata: serde_json::json!({
                "email_id": c.email_id,
                "chunk_index": c.chunk_index,
                "subject": c.subject,
                "from": c.from,
                "date": c.date.to_rfc3339(),
            }),
        }).collect();

        let stats = pipeline.index_chunks(index_name, &rag_chunks)
            .await
            .map_err(|e| EmailError::Rag(format!("RAG indexing failed: {}", e)))?;

        Ok(IndexStats {
            vectors_indexed: stats.indexed_count,
        })
    }

    #[cfg(not(feature = "rag"))]
    {
        // Fallback: just count
        Ok(IndexStats {
            vectors_indexed: chunks.len(),
        })
    }
}

/// Index statistics
struct IndexStats {
    vectors_indexed: usize,
}
