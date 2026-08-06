-- Adds embedding provenance tracking to `rag_collections` (Phase 12 close-out,
-- Track 2): `embedding_source`/`embedding_version` back
-- `CollectionSpec::embedding_source`/`embedding_version` (see
-- `crates/hacienda-rag/src/types.rs`), populated by a successful
-- `POST /v1/rag/collections/{name}/migrate-embeddings` job
-- (`PgVectorStore::set_embedding_provenance`).
--
-- Numbered 0101 (this crate's next free slot in its 0100-0199 block — see
-- `hacienda-core/migrations/README.md`).

ALTER TABLE rag_collections
    ADD COLUMN IF NOT EXISTS embedding_source  TEXT,
    ADD COLUMN IF NOT EXISTS embedding_version INTEGER NOT NULL DEFAULT 1;
