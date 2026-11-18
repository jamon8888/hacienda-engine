use crate::backend::EmailClient;
use crate::config::EmailConfig;
use crate::error::{EmailError, Result};
use crate::models::{EmailOperation, EmailSource, RagIndexStats};

/// Prepare email content for RAG indexing.
///
/// Chunking uses `hacienda-rag`'s real `chunk_full_text` (xberg's chunker).
/// The plugin does not own an embedding backend — the returned stats report
/// the chunks prepared, with `vectors_indexed == 0` until a caller embeds and
/// upserts them through hacienda-rag's stores. This is deliberately honest
/// about what is produced rather than claiming a phantom `RagPipeline` indexed
/// them.
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

    let mut total_chunks = 0usize;
    let mut errors = Vec::new();

    for envelope in envelopes {
        match client.get_message("INBOX", &envelope.id).await {
            Ok(message) => {
                let text = extract_text_for_rag(&message, &envelope);
                match chunk(&text, chunk_sz, config.rag.chunk_overlap) {
                    Ok(chunks) => total_chunks += chunks.len(),
                    Err(e) => errors.push(format!("Chunking failed for {}: {}", envelope.id, e)),
                }
            }
            Err(e) => errors.push(format!("Failed to fetch {}: {}", envelope.id, e)),
        }
    }

    Ok(crate::models::EmailResponse {
        success: errors.is_empty(),
        processed_count: total_chunks,
        rag_stats: Some(RagIndexStats {
            chunks_created: total_chunks,
            vectors_indexed: 0, // embedding/upsert happens in the RAG layer
            index_name: index,
        }),
        errors,
        ..Default::default()
    })
}

/// Fetch envelopes for RAG indexing.
async fn fetch_envelopes(
    client: &dyn EmailClient,
    source: &EmailSource,
) -> Result<Vec<crate::models::Envelope>> {
    match source {
        EmailSource::Account { name: _, mailbox } => {
            let mb = mailbox.as_deref().unwrap_or("INBOX");
            client.list_envelopes(mb, Some(5000)).await
        }
        EmailSource::Maildir { path: _ } => client.list_envelopes("INBOX", Some(5000)).await,
        EmailSource::Files { paths: _ } => {
            Err(EmailError::InvalidOperation("File source not yet implemented".to_string()))
        }
        EmailSource::Stdin => {
            Err(EmailError::InvalidOperation("Stdin source not yet implemented".to_string()))
        }
    }
}

/// Extract text for RAG with email metadata.
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
        parts.push(format!("Content:\n{text}"));
    } else if let Some(html) = &message.html_body {
        parts.push(format!("Content:\n{}", crate::ner::html_to_text(html)));
    }

    parts.join("\n\n")
}

/// Format addresses
fn format_addresses(addresses: &[crate::models::EmailAddress]) -> String {
    addresses
        .iter()
        .map(|a| match &a.name {
            Some(name) => format!("{name} <{}>", a.address),
            None => a.address.clone(),
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// Chunk text through hacienda-rag's real chunker when the `rag` feature is on,
/// falling back to a simple character-window chunker otherwise.
fn chunk(text: &str, chunk_size: usize, overlap: usize) -> Result<Vec<String>> {
    #[cfg(feature = "rag")]
    {
        use hacienda_rag::chunk_full_text;
        use xberg::chunking::{ChunkerType, ChunkingConfig};

        let config = ChunkingConfig {
            max_characters: chunk_size,
            overlap,
            trim: true,
            chunker_type: ChunkerType::Text,
            embedding: None,
            ..Default::default()
        };
        let records = chunk_full_text(text, &config)
            .map_err(|e| EmailError::Rag(format!("chunking failed: {e}")))?;
        Ok(records.into_iter().map(|r| r.content).collect())
    }

    #[cfg(not(feature = "rag"))]
    {
        if text.len() <= chunk_size {
            return Ok(vec![text.to_string()]);
        }
        let mut chunks = Vec::new();
        let mut start = 0;
        while start < text.len() {
            let end = (start + chunk_size).min(text.len());
            // Avoid slicing through a UTF-8 code point.
            let mut boundary = end;
            while boundary > start && !text.is_char_boundary(boundary) {
                boundary -= 1;
            }
            chunks.push(text[start..boundary].to_string());
            if boundary == text.len() {
                break;
            }
            start = boundary.saturating_sub(overlap);
        }
        Ok(chunks)
    }
}
