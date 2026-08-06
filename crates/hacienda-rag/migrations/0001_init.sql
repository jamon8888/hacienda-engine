-- hacienda-rag pgvector schema (Phase 12).
-- Collections, documents, and their embedded chunks for the `PgVectorStore`
-- backend (`src/backends/pgvector.rs`). Mirrors hacienda-core's
-- `store/postgres` migration convention: one embedded `sqlx::migrate!`
-- directory per crate, run explicitly (never implicitly from a constructor).

CREATE EXTENSION IF NOT EXISTS vector;

-- ============================================================================
-- COLLECTIONS
-- ============================================================================

CREATE TABLE IF NOT EXISTS rag_collections (
    name            TEXT PRIMARY KEY,
    embedding_dim   INTEGER NOT NULL,
    -- cosine | l2 | inner_product (DistanceMetric)
    distance_metric TEXT NOT NULL,
    -- flat | hnsw | diskann (IndexMethod; diskann is substituted with hnsw at
    -- index-build time -- see backends/pgvector.rs's `build_index`)
    index_method    TEXT NOT NULL,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- ============================================================================
-- DOCUMENTS
-- ============================================================================

CREATE TABLE IF NOT EXISTS rag_documents (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    collection      TEXT NOT NULL REFERENCES rag_collections(name) ON DELETE CASCADE,
    external_id     TEXT,
    title           TEXT,
    mime            TEXT,
    source_uri      TEXT,
    full_text       TEXT NOT NULL DEFAULT '',
    keywords        JSONB NOT NULL DEFAULT '[]'::jsonb,
    entities        JSONB NOT NULL DEFAULT 'null'::jsonb,
    labels          JSONB NOT NULL DEFAULT 'null'::jsonb,
    metadata        JSONB NOT NULL DEFAULT 'null'::jsonb,
    -- Generated tsvector for full-text/hybrid retrieval over doc.full_text.
    full_text_tsv   TSVECTOR GENERATED ALWAYS AS (to_tsvector('english', coalesce(full_text, ''))) STORED,
    ingested_at     TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- Idempotent upsert identity: `RagStore::upsert_document` replaces the
-- existing document (and its chunks) when `external_id` repeats within a
-- collection, matching `InMemoryVectorStore`'s `external_index` semantics.
CREATE UNIQUE INDEX IF NOT EXISTS idx_rag_documents_collection_external_id
    ON rag_documents (collection, external_id) WHERE external_id IS NOT NULL;
CREATE INDEX IF NOT EXISTS idx_rag_documents_collection ON rag_documents (collection);
CREATE INDEX IF NOT EXISTS idx_rag_documents_full_text_tsv ON rag_documents USING gin (full_text_tsv);

-- ============================================================================
-- CHUNKS
-- ============================================================================

CREATE TABLE IF NOT EXISTS rag_chunks (
    id               UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    document_id      UUID NOT NULL REFERENCES rag_documents(id) ON DELETE CASCADE,
    -- Denormalized from rag_documents.collection so retrieval/filter/index
    -- queries never need a join just to scope by collection.
    collection       TEXT NOT NULL REFERENCES rag_collections(name) ON DELETE CASCADE,
    external_id      TEXT,
    ordinal          INTEGER NOT NULL,
    content          TEXT NOT NULL,
    -- Unconstrained dimension: collections may have different embedding_dim.
    -- pgvector's HNSW builder rejects an index directly on an unconstrained
    -- `vector` column ("column does not have dimensions"), even restricted
    -- to one collection by a partial index's WHERE clause — verified against
    -- a live pgvector 0.8 instance. `build_index` (src/backends/pgvector.rs)
    -- and `retrieve_vector`'s query both cast to `vector(embedding_dim)`
    -- inside the indexed expression instead, which satisfies the builder
    -- without constraining this column (every collection's partial index
    -- casts to its own dimension).
    embedding        VECTOR NOT NULL,
    -- SparseVector / MultiVector (ChunkRecord's side vectors) are stored as
    -- JSONB pass-through, not native pgvector types: `RetrieveMode::Sparse`/
    -- `LateInteraction` are not implemented by this backend (see
    -- `PgVectorStore::capabilities`), so no query ever needs to decode these
    -- back into a scoreable form today. Kept so a future capability bump
    -- doesn't need a data migration.
    sparse_embedding JSONB,
    multi_vector     JSONB,
    chunk_metadata   JSONB NOT NULL DEFAULT 'null'::jsonb,
    -- Generated tsvector for full-text/hybrid retrieval over chunk.content.
    content_tsv      TSVECTOR GENERATED ALWAYS AS (to_tsvector('english', coalesce(content, ''))) STORED
);

CREATE INDEX IF NOT EXISTS idx_rag_chunks_collection ON rag_chunks (collection);
CREATE INDEX IF NOT EXISTS idx_rag_chunks_document_id ON rag_chunks (document_id);
CREATE INDEX IF NOT EXISTS idx_rag_chunks_content_tsv ON rag_chunks USING gin (content_tsv);

-- Per-collection ANN vector indexes (HNSW) are created/dropped dynamically by
-- `PgVectorStore::ensure_collection`/`drop_collection` as partial indexes
-- (`WHERE collection = ...`), because HNSW/IVFFlat need one fixed dimension
-- and one operator class (metric) per index, and both vary per collection.
-- `IndexMethod::Flat` collections get no ANN index (brute-force seq scan).
