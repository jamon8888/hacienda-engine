-- Hacienda Postgres schema (Phase 9)
-- Tables for audit chain, review queue, jobs, document versions, presets, and API keys.

-- ============================================================================
-- AUDIT CHAIN
-- ============================================================================

CREATE TABLE IF NOT EXISTS audit_segments (
    segment_id      UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    node_id         TEXT NOT NULL,
    config_hash     TEXT NOT NULL,
    prev_seal_hash  TEXT,  -- NULL for genesis segment
    sealed_tip      TEXT,  -- NULL while segment is open
    -- blake3 over every other SegmentSeal field (see segment::compute_seal_hash);
    -- NULL while the segment is open, set atomically with sealed_at/sealed_tip.
    seal_hash       TEXT,
    entry_count     BIGINT NOT NULL DEFAULT 0,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),  -- SegmentSeal::opened_at
    sealed_at       TIMESTAMPTZ  -- NULL while segment is open
);

CREATE TABLE IF NOT EXISTS audit_entries (
    id              TEXT PRIMARY KEY,  -- UUID v4 as string
    segment_id      UUID NOT NULL REFERENCES audit_segments(segment_id) ON DELETE CASCADE,
    sequence_num    BIGINT NOT NULL,   -- 1-based within segment
    category        TEXT NOT NULL,
    action          TEXT NOT NULL,     -- mask, hash, pseudonymize, remove, reveal, custom:<template>
    span_hash       TEXT NOT NULL,     -- blake3(plaintext) hex
    span_length     BIGINT NOT NULL,
    confidence      DOUBLE PRECISION,
    source          TEXT NOT NULL,     -- regex, model
    pipeline_version TEXT NOT NULL,
    config_hash     TEXT NOT NULL,
    principal       TEXT,              -- NULL for in-process callers
    -- blake3 chain hash linking this entry to its predecessor (entry::compute_chain_hash).
    -- Distinct from span_hash: chain_hash covers id/category/action/span_hash/config_hash/
    -- principal plus the previous entry's chain_hash, and is what AuditChain::verify checks.
    chain_hash      TEXT NOT NULL,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),  -- AuditEntry::timestamp
    UNIQUE (segment_id, sequence_num)
);

CREATE INDEX IF NOT EXISTS idx_audit_entries_segment_sequence
    ON audit_entries (segment_id, sequence_num);

-- ============================================================================
-- REVIEW QUEUE
-- ============================================================================

CREATE TABLE IF NOT EXISTS review_items (
    id              TEXT PRIMARY KEY,  -- UUID v4 as string
    text_snippet    TEXT NOT NULL,
    category        TEXT NOT NULL,
    start_pos       BIGINT NOT NULL,
    end_pos         BIGINT NOT NULL,
    confidence      DOUBLE PRECISION NOT NULL,
    source          TEXT NOT NULL,
    status          TEXT NOT NULL,     -- pending, in_review, approved, rejected, modified
    priority        TEXT NOT NULL DEFAULT 'high',  -- low, normal, high, critical
    assigned_reviewer TEXT,
    decided_by      TEXT,
    decided_at      TIMESTAMPTZ,
    decision        TEXT,              -- approve, reject, modify
    comment         TEXT,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    deadline        TIMESTAMPTZ
);

CREATE INDEX IF NOT EXISTS idx_review_items_status ON review_items (status);
CREATE INDEX IF NOT EXISTS idx_review_items_created ON review_items (created_at);

-- ============================================================================
-- JOBS
-- ============================================================================

CREATE TABLE IF NOT EXISTS jobs (
    id              TEXT PRIMARY KEY,  -- UUID v4 as string
    status          TEXT NOT NULL,     -- queued, running, succeeded, failed
    owner           TEXT,              -- NULL for trusted in-process callers
    result_json     TEXT,              -- JSON payload when succeeded
    error           TEXT,              -- Error message when failed
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_jobs_status ON jobs (status);
CREATE INDEX IF NOT EXISTS idx_jobs_owner ON jobs (owner);
CREATE INDEX IF NOT EXISTS idx_jobs_created ON jobs (created_at);

-- ============================================================================
-- DOCUMENT VERSIONS
-- ============================================================================

CREATE TABLE IF NOT EXISTS document_versions (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    document_id     UUID NOT NULL,
    version_sequence BIGINT NOT NULL,   -- 1-based, server-assigned via MAX+1 in same transaction
    content_hash    TEXT NOT NULL,      -- blake3 of content
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (document_id, version_sequence)
);

CREATE INDEX IF NOT EXISTS idx_document_versions_doc_id ON document_versions (document_id);

-- ============================================================================
-- PRESETS
-- ============================================================================

CREATE TABLE IF NOT EXISTS presets (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    name            TEXT NOT NULL UNIQUE,
    config_json     JSONB NOT NULL,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- ============================================================================
-- API KEYS
-- ============================================================================

-- `key_hash` (Argon2id) is salted per row and exists purely for verification — it
-- cannot be recomputed from a presented key, so it is not a usable lookup key.
-- `lookup_hash` is a deterministic BLAKE3 digest of the raw key, computed once at
-- issuance and again on every presented key, so it is the only column a resolver can
-- equality-match against. See `hacienda-core/src/auth/keys.rs` module docs.
CREATE TABLE IF NOT EXISTS api_keys (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    key_hash        TEXT NOT NULL UNIQUE,  -- Argon2id hash (verification only)
    lookup_hash     TEXT NOT NULL UNIQUE,  -- BLAKE3 digest (deterministic lookup)
    owner           TEXT NOT NULL,
    capabilities    JSONB NOT NULL,         -- array of capability strings
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    revoked_at      TIMESTAMPTZ             -- NULL = active
);

CREATE INDEX IF NOT EXISTS idx_api_keys_owner ON api_keys (owner);
CREATE INDEX IF NOT EXISTS idx_api_keys_revoked ON api_keys (revoked_at) WHERE revoked_at IS NOT NULL;
-- No separate index on lookup_hash: the UNIQUE constraint above already creates one.