//! Server-side text chunking for documents submitted without pre-chunked content.
//!
//! `POST .../collections/{name}/documents` previously required every caller to submit
//! `chunks` already split and embedded — `DocumentRecord.full_text` was stored for
//! full-text search only, never chunked. This module fills that gap: it wraps `xberg`'s
//! chunker (behind this crate's `chunking` feature) to turn `full_text` into
//! [`ChunkRecord`]s with `content` set and `embedding` left empty, for a caller to embed
//! separately or leave unembedded (a collection used only for full-text/keyword
//! retrieval has no need of one).

use crate::error::{RagError, RagResult};
use crate::types::ChunkRecord;
use xberg::chunking::{chunk_text, ChunkingConfig};

/// Split `full_text` into [`ChunkRecord`]s using `xberg`'s chunker.
///
/// Chunks are ordered by their position in `full_text`, numbered from zero via
/// `ordinal`. `embedding` is left empty (`Vec::new()`) — this function does no
/// embedding of its own; a caller that wants embedded chunks runs them through an
/// embedder (e.g. `xberg::embed_texts_async`, already used by `migrate_embeddings`)
/// before upserting. `tenant_id`, when `Some`, is stamped on every produced
/// chunk so retrieval can scope by tenant without joining the parent document.
///
/// # Errors
///
/// Returns [`RagError::Chunking`], not a bare [`RagError::Core`] — see that
/// variant's doc comment for why this call site is worth naming explicitly.
pub fn chunk_full_text(
    full_text: &str,
    config: &ChunkingConfig,
    tenant_id: Option<&str>,
) -> RagResult<Vec<ChunkRecord>> {
    let result = chunk_text(full_text, config, None).map_err(|source| RagError::Chunking {
        input_chars: full_text.chars().count(),
        source: Box::new(source),
    })?;

    Ok(result
        .chunks
        .into_iter()
        .enumerate()
        .map(|(ordinal, chunk)| ChunkRecord {
            external_id: None,
            ordinal: ordinal as u32,
            content: chunk.content,
            embedding: Vec::new(),
            sparse_embedding: None,
            multi_vector: None,
            chunk_metadata: serde_json::Value::Null,
            // Chunking does not know the parent document's dossier/version yet;
            // those stay `None` until the caller (upsert) propagates them from
            // the `DocumentRecord`. Same for `tenant_id`: stamped here when
            // provided, otherwise left `None` (default tenant).
            dossier_id: None,
            version_ref: None,
            tenant_id: tenant_id.map(str::to_string),
            acl: None,
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn should_split_long_text_into_multiple_ordered_chunks() {
        let config = ChunkingConfig {
            max_characters: 50,
            overlap: 0,
            ..Default::default()
        };
        let text = "Sentence one. ".repeat(20);

        let chunks = chunk_full_text(&text, &config, None).expect("chunking should succeed");

        assert!(
            chunks.len() > 1,
            "expected more than one chunk, got {}",
            chunks.len()
        );
        for (i, chunk) in chunks.iter().enumerate() {
            assert_eq!(chunk.ordinal, i as u32);
            assert!(
                chunk.embedding.is_empty(),
                "chunker must not embed on its own"
            );
            assert!(!chunk.content.is_empty());
        }
    }

    #[test]
    fn should_return_a_single_chunk_for_short_text() {
        let config = ChunkingConfig::default();
        let chunks = chunk_full_text("hello world", &config, None).expect("chunking should succeed");
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].content, "hello world");
    }
}
