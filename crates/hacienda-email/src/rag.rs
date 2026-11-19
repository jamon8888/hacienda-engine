use crate::backend::EmailClient;
use crate::config::EmailConfig;
use crate::error::{EmailError, Result};
use crate::models::{EmailOperation, EmailSource, Envelope, RagIndexStats};
use crate::pii::PiiEngine;
use hacienda_rag::{ChunkRecord, CollectionSpec, DocumentRecord, RagStore};

/// Runtime RAG persistence, injected by the caller (the HTTP API passes its
/// `ApiState::rag_store`). This is the same `RagStore` document ingestion uses,
/// so emails land in a collection queryable through the existing `/v1/rag/*`
/// retrieve endpoints instead of being merely chunked in memory.
#[derive(Clone, Copy)]
pub struct RagIngestion<'a> {
    pub store: &'a dyn RagStore,
    /// Legal dossier used for retrieval scoping (mirrors document ingestion).
    pub dossier_id: Option<&'a str>,
    pub account: &'a str,
}

/// Upsert one redacted email as a document with its chunks.
///
/// Deliberately mirrors the document ingestion path: `chunk_full_text` output
/// is passed to `upsert_document` with BYOV (empty) embeddings, and the
/// `external_id` makes re-ingestion idempotent. Only already-redacted text is
/// ever written (design D2).
async fn upsert_email(
    rag: &RagIngestion<'_>,
    collection: &str,
    envelope: &Envelope,
    redacted_text: &str,
    chunks: &[String],
) -> Result<usize> {
    let store = rag.store;
    if store
        .get_collection(collection)
        .await
        .map_err(|e| EmailError::Rag(format!("get collection failed: {e}")))?
        .is_none()
    {
        // BYOV collection: chunks carry no dense vector, exactly like the
        // document API's `chunk_full_text` + `upsert_document` path.
        store
            .ensure_collection(&CollectionSpec::new(collection, 0))
            .await
            .map_err(|e| EmailError::Rag(format!("ensure collection failed: {e}")))?;
    }

    let mailbox = if envelope.mailbox.is_empty() {
        "INBOX"
    } else {
        &envelope.mailbox
    };
    let external_id = format!("email:{}:{}:{}", rag.account, mailbox, envelope.id);
    let document = DocumentRecord {
        external_id: Some(external_id.clone()),
        title: Some(envelope.subject.clone()),
        mime: Some("message/rfc822".to_string()),
        source_uri: Some(format!("imap://{}/{}", rag.account, envelope.id)),
        full_text: redacted_text.to_string(),
        keywords: Vec::new(),
        entities: serde_json::Value::Null,
        labels: serde_json::Value::Null,
        metadata: serde_json::json!({
            "account": rag.account,
            "mailbox": mailbox,
            "subject": envelope.subject,
            "privacy": "redacted",
        }),
        dossier_id: rag.dossier_id.map(str::to_string),
        version_ref: None,
    };
    let records: Vec<ChunkRecord> = chunks
        .iter()
        .enumerate()
        .map(|(ordinal, content)| ChunkRecord {
            external_id: Some(format!("{external_id}:chunk:{ordinal}")),
            ordinal: ordinal as u32,
            content: content.clone(),
            embedding: Vec::new(),
            sparse_embedding: None,
            multi_vector: None,
            chunk_metadata: serde_json::Value::Null,
            dossier_id: rag.dossier_id.map(str::to_string),
            version_ref: None,
        })
        .collect();
    let count = records.len();
    store
        .upsert_document(collection, &document, &records)
        .await
        .map_err(|e| EmailError::Rag(format!("upsert failed: {e}")))?;
    Ok(count)
}

/// Prepare and optionally persist email content for RAG indexing.
///
/// Chunking uses `hacienda-rag`'s real `chunk_full_text` (xberg's chunker).
/// When the caller injects a `RagIngestion`, the redacted chunks are upserted
/// into that `RagStore` with the same `external_id`/`dossier_id` contract as
/// document ingestion, and `vectors_indexed` reports the persisted count.
/// Without a store, the plugin still chunks but reports `vectors_indexed == 0`
/// — it never claims to have indexed what it only prepared.
pub async fn index_for_rag(
    client: &dyn EmailClient,
    pii: &PiiEngine,
    op: EmailOperation,
    config: &EmailConfig,
    rag: Option<RagIngestion<'_>>,
) -> Result<crate::models::EmailResponse> {
    let EmailOperation::IndexForRag {
        source,
        index_name,
        chunk_size,
    } = op
    else {
        return Err(EmailError::InvalidOperation(
            "Expected IndexForRag operation".to_string(),
        ));
    };

    let envelopes = fetch_envelopes(client, &source).await?;
    let index = index_name.unwrap_or_else(|| config.rag.index_name.clone());
    let chunk_sz = chunk_size.unwrap_or(config.rag.chunk_size);

    let mut total_chunks = 0usize;
    let mut total_indexed = 0usize;
    let mut errors = Vec::new();

    for envelope in envelopes {
        match client.get_message("INBOX", &envelope.id).await {
            Ok(message) => {
                let text = extract_text_for_rag(&message, &envelope);
                // D2 (ingestion guard): redact BEFORE chunking/embedding. Fail closed —
                // if the PII engine errors, do not index this email's plaintext.
                let safe_text = match pii
                    .redact(
                        &text,
                        &config.redaction.rules,
                        &config.redaction.replacement,
                        config.redaction.preserve_format,
                    )
                    .await
                {
                    Ok((redacted, _)) => redacted,
                    Err(e) => {
                        errors.push(format!(
                            "Redaction failed for {} — refusing to index (fail-closed): {e}",
                            envelope.id
                        ));
                        continue;
                    }
                };
                match chunk(&safe_text, chunk_sz, config.rag.chunk_overlap) {
                    Ok(chunks) => {
                        total_chunks += chunks.len();
                        // Same ingestion logic as documents: persist the redacted
                        // chunks through the injected RagStore when one is configured.
                        if let Some(rag) = &rag {
                            match upsert_email(rag, &index, &envelope, &safe_text, &chunks).await {
                                Ok(count) => total_indexed += count,
                                Err(e) => errors
                                    .push(format!("RAG upsert failed for {}: {e}", envelope.id)),
                            }
                        }
                    }
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
            vectors_indexed: total_indexed,
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
        EmailSource::Files { paths: _ } => Err(EmailError::InvalidOperation(
            "File source not yet implemented".to_string(),
        )),
        EmailSource::Stdin => Err(EmailError::InvalidOperation(
            "Stdin source not yet implemented".to_string(),
        )),
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
    parts.push(format!(
        "Date: {}",
        envelope.date.format("%Y-%m-%d %H:%M:%S UTC")
    ));

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

#[cfg(test)]
mod tests {
    use super::*;
    use hacienda_rag::InMemoryVectorStore;

    fn envelope() -> Envelope {
        Envelope {
            id: "uid-1".to_string(),
            mailbox: "INBOX".to_string(),
            subject: "Test".to_string(),
            from: Vec::new(),
            to: Vec::new(),
            date: chrono::Utc::now(),
            flags: Vec::new(),
            has_attachments: false,
        }
    }

    #[tokio::test]
    async fn upsert_email_persists_dossier_scoped_chunks() {
        let store = InMemoryVectorStore::new("email-test");
        let collection = "email-corpus";
        store
            .ensure_collection(&CollectionSpec::new(collection, 0))
            .await
            .unwrap();

        let rag = RagIngestion {
            store: &store,
            dossier_id: Some("dossier-42"),
            account: "work",
        };
        let chunks = vec!["chunk one".to_string(), "chunk two".to_string()];
        let count = upsert_email(&rag, collection, &envelope(), "[REDACTED] body", &chunks)
            .await
            .unwrap();
        assert_eq!(count, 2);

        let (docs, _) = store.list_documents(collection, 10, 0).await.unwrap();
        assert_eq!(docs.len(), 1);
        assert_eq!(
            docs[0].external_id.as_deref(),
            Some("email:work:INBOX:uid-1")
        );
        assert_eq!(docs[0].dossier_id.as_deref(), Some("dossier-42"));

        let (record, stored_chunks) = store
            .get_document_chunks(collection, &docs[0].id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(record.full_text, "[REDACTED] body");
        assert_eq!(stored_chunks.len(), 2);
        assert!(stored_chunks
            .iter()
            .all(|c| c.dossier_id.as_deref() == Some("dossier-42")));

        // Re-ingesting the same UID is idempotent (stable external_id).
        upsert_email(&rag, collection, &envelope(), "[REDACTED] body", &chunks)
            .await
            .unwrap();
        let (docs_again, _) = store.list_documents(collection, 10, 0).await.unwrap();
        assert_eq!(docs_again.len(), 1);
    }
}
