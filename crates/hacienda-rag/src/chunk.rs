//! Server-side text chunking for documents submitted without pre-chunked content.
//!
//! `POST .../collections/{name}/documents` previously required every caller to submit
//! `chunks` already split and embedded — `DocumentRecord.full_text` was stored for
//! full-text search only, never chunked. This module fills that gap: it wraps `xberg`'s
//! chunker (behind this crate's `chunking` feature) to turn `full_text` into
//! [`ChunkRecord`]s with `content` set and `embedding` left empty, for a caller to embed
//! separately or leave unembedded (a collection used only for full-text/keyword
//! retrieval has no need of one).

use crate::error::RagResult;
use crate::types::ChunkRecord;
use xberg::chunking::{chunk_text, ChunkingConfig};

/// Split `full_text` into [`ChunkRecord`]s using `xberg`'s chunker.
///
/// Chunks are ordered by their position in `full_text`, numbered from zero via
/// `ordinal`. `embedding` is left empty (`Vec::new()`) — this function does no
/// embedding of its own; a caller that wants embedded chunks runs them through an
/// embedder (e.g. `xberg::embed_texts_async`, already used by `migrate_embeddings`)
/// before upserting.
pub fn chunk_full_text(full_text: &str, config: &ChunkingConfig) -> RagResult<Vec<ChunkRecord>> {
    let result = chunk_text(full_text, config, None)?;

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

        let chunks = chunk_full_text(&text, &config).expect("chunking should succeed");

        assert!(chunks.len() > 1, "expected more than one chunk, got {}", chunks.len());
        for (i, chunk) in chunks.iter().enumerate() {
            assert_eq!(chunk.ordinal, i as u32);
            assert!(chunk.embedding.is_empty(), "chunker must not embed on its own");
            assert!(!chunk.content.is_empty());
        }
    }

    #[test]
    fn should_return_a_single_chunk_for_short_text() {
        let config = ChunkingConfig::default();
        let chunks = chunk_full_text("hello world", &config).expect("chunking should succeed");
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].content, "hello world");
    }
}
