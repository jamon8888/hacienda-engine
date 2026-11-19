use crate::backend::EmailClient;
use crate::config::EmailConfig;
use crate::error::{EmailError, Result};
use crate::models::{EmailOperation, EmailSource, Envelope, RagIndexStats};
use crate::pii::PiiEngine;
use futures::stream::{FuturesUnordered, StreamExt};
use hacienda_rag::{ChunkRecord, CollectionSpec, DocumentRecord, RagStore};
use std::collections::HashSet;

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
    /// Envelopes per IMAP page (bounded memory for large mailboxes).
    pub page_size: u32,
    /// Stop after this many pages; 0 = unlimited.
    pub max_pages: u32,
    /// Bound on concurrent redaction/chunking tasks after fetch.
    pub concurrency: usize,
    /// Skip already-ingested messages (remembered watermark, key `watermark:<account>:<mailbox>`).
    pub incremental: bool,
    /// Run `RagStore::optimize()` once at the end of the run.
    pub optimize_after_run: bool,
    /// Set false for a single-shot call (unit tests) to skip the watermark write.
    pub persist_watermark: bool,
}

/// Build the document + chunks for one redacted email.
///
/// Deliberately mirrors the document ingestion path: chunks carry BYOV (empty)
/// embeddings and the `external_id` makes re-ingestion idempotent. Only
/// already-redacted text is ever written (design D2).
fn build_email_records(
    rag: &RagIngestion<'_>,
    envelope: &Envelope,
    redacted_text: &str,
    chunks: &[String],
) -> (DocumentRecord, Vec<ChunkRecord>) {
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
    (document, records)
}

/// Watermark collection for incremental ingestion (one row per account/mailbox).
const WATERMARK_COLLECTION: &str = "email-watermarks";

fn watermark_external_id(account: &str, mailbox: &str) -> String {
    format!("watermark:{account}:{mailbox}")
}

/// Numeric UID of an envelope, when the backend exposes one (IMAP does).
fn envelope_uid(envelope: &Envelope) -> Option<u64> {
    envelope.id.trim().parse::<u64>().ok()
}

/// Highest already-ingested UID for this account+mailbox, or 0 when unknown.
async fn read_watermark(rag: &RagIngestion<'_>) -> u64 {
    let store = rag.store;
    if store
        .get_collection(WATERMARK_COLLECTION)
        .await
        .unwrap_or(None)
        .is_none()
    {
        return 0;
    }
    let (docs, _total) = store
        .list_documents(WATERMARK_COLLECTION, 500, 0)
        .await
        .unwrap_or_default();
    for doc in docs {
        if doc.external_id.as_deref() == Some(&watermark_external_id(rag.account, "INBOX")) {
            // The UID lives in the document metadata.
            if let Some(uid) = doc.metadata.get("last_uid").and_then(|v| v.as_u64()) {
                return uid;
            }
        }
    }
    0
}

/// Remember the highest UID ingested in this run (one row, upserted).
async fn write_watermark(rag: &RagIngestion<'_>, watermark: u64) -> Result<()> {
    let store = rag.store;
    if store
        .get_collection(WATERMARK_COLLECTION)
        .await
        .map_err(|e| EmailError::Rag(format!("get collection failed: {e}")))?
        .is_none()
    {
        store
            .ensure_collection(&CollectionSpec::new(WATERMARK_COLLECTION, 0))
            .await
            .map_err(|e| EmailError::Rag(format!("ensure collection failed: {e}")))?;
    }
    let doc = DocumentRecord {
        external_id: Some(watermark_external_id(rag.account, "INBOX")),
        title: Some("email ingestion watermark".to_string()),
        full_text: "watermark".to_string(),
        metadata: serde_json::json!({ "last_uid": watermark }),
        ..Default::default()
    };
    store
        .upsert_document(WATERMARK_COLLECTION, &doc, &[])
        .await
        .map_err(|e| EmailError::Rag(format!("upsert watermark failed: {e}")))?;
    Ok(())
}

/// Ensure the target collection exists (BYOV, dim 0 — same as the document API).
async fn ensure_collection(rag: &RagIngestion<'_>, collection: &str) -> Result<()> {
    if rag
        .store
        .get_collection(collection)
        .await
        .map_err(|e| EmailError::Rag(format!("get collection failed: {e}")))?
        .is_none()
    {
        rag.store
            .ensure_collection(&CollectionSpec::new(collection, 0))
            .await
            .map_err(|e| EmailError::Rag(format!("ensure collection failed: {e}")))?;
    }
    Ok(())
}

/// Upsert one redacted email as a document with its chunks (single-message path).
#[allow(dead_code)]
async fn upsert_email(
    rag: &RagIngestion<'_>,
    collection: &str,
    envelope: &Envelope,
    redacted_text: &str,
    chunks: &[String],
) -> Result<usize> {
    ensure_collection(rag, collection).await?;
    let (document, records) = build_email_records(rag, envelope, redacted_text, chunks);
    let count = records.len();
    rag.store
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

    let index = index_name.unwrap_or_else(|| config.rag.index_name.clone());
    let chunk_sz = chunk_size.unwrap_or(config.rag.chunk_size);
    let mailbox = match &source {
        EmailSource::Account { mailbox, .. } => {
            mailbox.clone().unwrap_or_else(|| "INBOX".to_string())
        }
        _ => "INBOX".to_string(),
    };
    let mut seen: HashSet<String> = HashSet::new();

    let (page_size, max_pages, concurrency, incremental, optimize_after_run, persist) = match &rag {
        Some(r) => (
            r.page_size,
            r.max_pages,
            r.concurrency,
            r.incremental,
            r.optimize_after_run,
            r.persist_watermark,
        ),
        None => (
            config.ingestion.page_size,
            0,
            config.ingestion.concurrency,
            config.ingestion.incremental,
            config.ingestion.optimize_after_run,
            false,
        ),
    };

    let mut total_chunks = 0usize;
    let mut total_indexed = 0usize;
    let mut errors: Vec<String> = Vec::new();
    let mut last_uid: u64 = 0;

    // Inbox is the canonical mailbox for highlighting point 1.
    if let Some(rag) = &rag {
        ensure_collection(rag, &index).await?;
        if incremental {
            last_uid = read_watermark(rag).await;
        }
    }

    let mut page = 1u32;
    loop {
        let envelopes = fetch_envelopes(client, &source, page, page_size).await?;
        if envelopes.is_empty() {
            break;
        }
        // ignore messages we already ingested in a previous run
        let mut selected: Vec<Envelope> = Vec::with_capacity(envelopes.len());
        for envelope in &envelopes {
            let uid = envelope_uid(envelope);
            // Skip messages already ingested in a previous run (watermark) and
            // duplicates already handled inside this run.
            if uid.is_some_and(|uid| uid <= last_uid) || seen.contains(envelope.id.as_str()) {
                continue;
            }
            seen.insert(envelope.id.clone());
            if let Some(uid) = uid {
                last_uid = last_uid.max(uid);
            }
            selected.push(envelope.clone());
        }

        if !selected.is_empty() {
            // Fetch, redact, chunk with bounded concurrency. The email client is
            // behind a Mutex, so IMAP fetches stay serialized; this only
            // parallelizes post-fetch redaction/chunking.
            let spawn_task = |envelope: Envelope| {
                let mailbox = mailbox.clone();
                async move {
                    let message = match client.get_message(&mailbox, &envelope.id).await {
                        Ok(message) => message,
                        Err(e) => {
                            return (
                                envelope.id.clone(),
                                None,
                                Some(format!("Failed to fetch {}: {e}", envelope.id)),
                            )
                        }
                    };
                    let text = extract_text_for_rag(&message, &envelope);
                    // D2 (ingestion guard): redact BEFORE chunking. Fail closed — if
                    // the PII engine errors, this message is never indexed.
                    match pii
                        .redact(
                            &text,
                            &config.redaction.rules,
                            &config.redaction.replacement,
                            config.redaction.preserve_format,
                        )
                        .await
                    {
                        Ok((redacted, _)) => {
                            let chunks = chunk(&redacted, chunk_sz, config.rag.chunk_overlap);
                            (
                                envelope.id.clone(),
                                Some((envelope, redacted, chunks)),
                                None,
                            )
                        }
                        Err(e) => (
                            envelope.id.clone(),
                            None,
                            Some(format!(
                                "Redaction failed for {} — refusing to index (fail-closed): {e}",
                                envelope.id
                            )),
                        ),
                    }
                }
            };

            let mut outputs = Vec::with_capacity(selected.len());
            let mut queue = FuturesUnordered::new();
            let mut next = 0usize;
            let bound = concurrency.max(1).min(selected.len());
            while next < bound {
                queue.push(spawn_task(selected[next].clone()));
                next += 1;
            }
            while let Some((id, payload, error)) = queue.next().await {
                if let Some(err) = error {
                    errors.push(err);
                } else if let Some((envelope, redacted, chunks)) = payload {
                    match chunks {
                        Ok(chunks) if chunks.is_empty() => {}
                        Ok(chunks) => outputs.push((envelope, redacted, chunks)),
                        Err(e) => errors.push(format!("Chunking failed for {}: {}", id, e)),
                    }
                }
                if next < selected.len() {
                    queue.push(spawn_task(selected[next].clone()));
                    next += 1;
                }
            }

            let mut prepared: Vec<(DocumentRecord, Vec<ChunkRecord>)> =
                Vec::with_capacity(outputs.len());
            for (envelope, redacted, chunks) in outputs {
                total_chunks += chunks.len();
                if let Some(rag) = &rag {
                    let (document, records) =
                        build_email_records(rag, &envelope, &redacted, &chunks);
                    prepared.push((document, records));
                }
            }

            // One batch upsert per page — the same call documents use.
            if let Some(rag) = &rag {
                if !prepared.is_empty() {
                    let chunk_count: usize = prepared.iter().map(|(_, c)| c.len()).sum();
                    match rag.store.upsert_documents(&index, &prepared).await {
                        Ok(_) => total_indexed += chunk_count,
                        Err(e) => errors.push(format!(
                            "RAG batch upsert failed for {} documents: {e}",
                            prepared.len()
                        )),
                    }
                }
            }
        }

        if (envelopes.len() as u32) < page_size {
            break;
        }
        if max_pages > 0 && page >= max_pages {
            break;
        }
        page += 1;
    }

    if let Some(rag) = &rag {
        if persist && last_uid > 0 {
            if let Err(e) = write_watermark(rag, last_uid).await {
                errors.push(format!("Failed to persist watermark: {e}"));
            }
        }
        if optimize_after_run {
            if let Err(e) = rag.store.optimize().await {
                errors.push(format!("Failed to optimize RAG store: {e}"));
            }
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

/// Fetch one page of envelopes for RAG indexing.
async fn fetch_envelopes(
    client: &dyn EmailClient,
    source: &EmailSource,
    page: u32,
    page_size: u32,
) -> Result<Vec<crate::models::Envelope>> {
    let mailbox = match source {
        EmailSource::Account { mailbox, .. } => mailbox.as_deref().unwrap_or("INBOX"),
        EmailSource::Maildir { path: _ } => "INBOX",
        EmailSource::Files { paths: _ } => {
            return Err(EmailError::InvalidOperation(
                "File source not yet implemented".to_string(),
            ))
        }
        EmailSource::Stdin => {
            return Err(EmailError::InvalidOperation(
                "Stdin source not yet implemented".to_string(),
            ))
        }
    };
    client.list_envelopes_page(mailbox, page, page_size).await
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
            page_size: 500,
            max_pages: 0,
            concurrency: 2,
            incremental: false,
            optimize_after_run: false,
            persist_watermark: false,
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

    #[tokio::test]
    async fn watermark_round_trips_and_filters_old_uids() {
        use hacienda_rag::InMemoryVectorStore;

        let store = InMemoryVectorStore::new("watermark-test");
        let rag = RagIngestion {
            store: &store,
            dossier_id: None,
            account: "work",
            page_size: 500,
            max_pages: 0,
            concurrency: 2,
            incremental: true,
            optimize_after_run: false,
            persist_watermark: true,
        };

        // No watermark present yet.
        assert_eq!(read_watermark(&rag).await, 0);

        // Highest UID only, not a count.
        let seen = vec![17u64, 42, 9, 3];
        let highest = seen.iter().copied().max().unwrap();
        write_watermark(&rag, highest).await.unwrap();
        assert_eq!(read_watermark(&rag).await, 42);

        // A new batch with lower UIDs must be skipped entirely.
        let mut ran = 0usize;
        for uid in seen {
            if uid <= read_watermark(&rag).await {
                continue;
            }
            ran += 1;
        }
        assert_eq!(ran, 0);

        // A batch with one newer UID must be processed exactly once.
        for uid in [42u64, 43, 43] {
            if uid <= read_watermark(&rag).await {
                continue;
            }
            ran += 1;
            write_watermark(&rag, uid).await.unwrap();
        }
        assert_eq!(ran, 1);
        assert_eq!(read_watermark(&rag).await, 43);
    }
}
