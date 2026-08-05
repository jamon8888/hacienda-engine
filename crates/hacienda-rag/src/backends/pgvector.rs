//! Postgres [`RagStore`] backend using the `pgvector` extension.
//!
//! Gated behind the `postgres` feature (mirrors `hacienda-core`'s own
//! `store::postgres` gating: `sqlx` + `PgPool`, migrations run explicitly via
//! [`PgVectorStore::migrate`], never implicitly from a constructor).
//!
//! # Distance metrics
//!
//! [`DistanceMetric`] maps to pgvector's distance operators: `Cosine` → `<=>`
//! (cosine distance), `L2` → `<->` (Euclidean distance), `InnerProduct` →
//! `<#>` (negative inner product). Scores are converted back to the
//! higher-is-better convention [`InMemoryVectorStore`](crate::InMemoryVectorStore)
//! uses (`1 - distance` for cosine, `-distance` for L2/inner-product) so a
//! caller comparing scores across backends sees the same ordering direction.
//!
//! # Index methods
//!
//! [`IndexMethod::Hnsw`] creates a partial HNSW index
//! (`CREATE INDEX ... USING hnsw (embedding <ops>) WHERE collection = ...`)
//! scoped to one collection, since HNSW requires one fixed dimension and one
//! operator class per index and both vary per collection.
//! [`IndexMethod::Diskann`] has no pgvector equivalent — pgvector ships HNSW
//! and IVFFlat, not DiskANN — so it is substituted with HNSW at index-build
//! time, with a `tracing::warn!`. Callers can discover this ahead of time via
//! [`PgVectorStore::capabilities`], which only ever advertises `Flat`/`Hnsw`
//! (never `Diskann`), per [`Capabilities`]' own contract as the mechanism for
//! a caller to discover backend limitations in advance.
//!
//! # Retrieval modes
//!
//! `Vector` uses the pgvector distance operator directly. `FullText` and
//! `Hybrid` use Postgres's native `tsvector`/`tsquery` full-text search
//! (`content_tsv`, a generated column — see `migrations/0001_init.sql`).
//! `Hybrid` fuses the vector and full-text candidate lists with reciprocal
//! rank fusion (RRF, `k = 60`). `Sparse` and `LateInteraction` are not
//! implemented by this backend (see [`PgVectorStore::capabilities`]) —
//! `sparse_embedding`/`multi_vector` are stored as JSONB pass-through so a
//! future capability bump does not need a data migration, but no query path
//! reads them back today.
//!
//! # Compile-time query checking
//!
//! Unlike `hacienda-core`'s `store::postgres` modules, this file deliberately
//! uses runtime-checked `sqlx::query`/`sqlx::query_as`/`sqlx::QueryBuilder`
//! throughout rather than the `sqlx::query!`/`query_as!` compile-time macros.
//! Two reasons: (1) `retrieve`/`delete_by_filter` compile arbitrary
//! [`Filter`] trees into SQL at runtime, which the compile-time macros cannot
//! express at all; (2) the compile-time macros require `DATABASE_URL` (a live,
//! already-migrated Postgres) to be reachable at `cargo build` time — verified
//! empirically against this workspace's existing `hacienda-core --features
//! postgres` build, which fails outright without it. Using the runtime API
//! uniformly keeps `cargo build -p hacienda-rag --features postgres` working
//! with no live database, at the cost of losing compile-time column-type
//! checking (mitigated by the `#[derive(sqlx::FromRow)]` row structs and this
//! module's own test suite).

use crate::capability::Capabilities;
use crate::error::{RagError, RagResult};
use crate::filter::{Filter, FilterNamespace, ParsedField};
use crate::query::{RetrieveMode, RetrieveOutput, RetrieveQuery};
use crate::store::RagStore;
use crate::types::{
    validate_chunk_side_vectors, ChunkId, ChunkRecord, CollectionSpec, CollectionStats,
    DistanceMetric, DocumentId, DocumentRecord, DocumentSummary, IndexMethod, PrimaryScore,
    RetrievedChunk,
};
use async_trait::async_trait;
use pgvector::Vector as PgVector;
use sqlx::postgres::PgRow;
use sqlx::{FromRow, PgPool, Postgres, QueryBuilder, Row};
use std::collections::HashSet;
use std::time::Instant;
use uuid::Uuid;

/// Reciprocal rank fusion constant for hybrid retrieval. `60` is the value
/// used in the original RRF paper (Cormack et al., 2009) and is the de facto
/// default across hybrid-search implementations; not tuned for this crate.
const RRF_K: f64 = 60.0;

/// Columns selected from a `rag_chunks c JOIN rag_documents d` pair, shared
/// by every retrieval mode.
const CHUNK_DOC_COLUMNS: &str =
    "c.id, c.document_id, c.ordinal, c.external_id, c.content, c.chunk_metadata, \
     d.external_id AS doc_external_id, d.title AS doc_title, d.mime AS doc_mime, \
     d.keywords AS doc_keywords, d.labels AS doc_labels, d.entities AS doc_entities, \
     d.metadata AS doc_metadata, d.ingested_at AS doc_ingested_at";

/// A Postgres [`RagStore`] backed by the `pgvector` extension.
pub struct PgVectorStore {
    name: String,
    pool: PgPool,
}

impl PgVectorStore {
    /// Create a store from an already-connected, already-migrated pool.
    /// Matches `hacienda-core`'s `Postgres*Store::new(pool)` convention: no
    /// implicit connection or migration inside the constructor.
    pub fn new(pool: PgPool) -> Self {
        Self::with_name(pool, "pgvector")
    }

    /// As [`Self::new`], with an explicit store name (used in
    /// [`RagError::UnsupportedMode`] messages).
    pub fn with_name(pool: PgPool, name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            pool,
        }
    }

    /// Connect to Postgres and return a pool, for callers that don't already
    /// have one. Mirrors `hacienda_core::store::postgres::connection::connect`.
    ///
    /// # Errors
    ///
    /// [`RagError::Backend`] if the pool cannot be established.
    pub async fn connect(database_url: &str) -> RagResult<PgPool> {
        sqlx::postgres::PgPoolOptions::new()
            .max_connections(10)
            .connect(database_url)
            .await
            .map_err(pg_err)
    }

    /// Run this crate's embedded migrations against `pool`. Call explicitly
    /// from the process entry point or test harness — never implicitly from
    /// [`Self::new`], matching `hacienda-core`'s Design Decision D4 (Phase 9).
    ///
    /// This crate's migrations are numbered `0100+` specifically so this can
    /// run against the same database as `hacienda_core::store::postgres`'s own
    /// migrations without a `_sqlx_migrations` version collision — see
    /// `hacienda-core/migrations/README.md`.
    ///
    /// `set_ignore_missing(true)`: the numbering split alone isn't sufficient.
    /// sqlx's `_sqlx_migrations` table is one per physical database, and
    /// `Migrator::run` rejects the whole table as soon as it contains any
    /// applied-migration row outside *this* migrator's own resolved list
    /// (`VersionMissing`) — which every `hacienda-core` migration row is, from
    /// this migrator's point of view. `ignore_missing` relaxes that check to
    /// "every version this migrator knows about must match its checksum",
    /// which is what the shared-database deployment actually needs.
    ///
    /// # Errors
    ///
    /// [`RagError::Backend`] if any migration fails.
    pub async fn migrate(pool: &PgPool) -> RagResult<()> {
        let mut migrator = sqlx::migrate!("./migrations");
        migrator.set_ignore_missing(true);
        migrator.run(pool).await.map_err(pg_migrate_err)
    }
}

fn pg_err(e: sqlx::Error) -> RagError {
    RagError::Backend(Box::new(e))
}

fn pg_migrate_err(e: sqlx::migrate::MigrateError) -> RagError {
    RagError::Backend(Box::new(e))
}

fn json_err(e: serde_json::Error) -> RagError {
    RagError::Backend(Box::new(e))
}

// ── metric / index-method <-> wire-format conversions ─────────────────────

fn metric_db_str(metric: DistanceMetric) -> &'static str {
    match metric {
        DistanceMetric::Cosine => "cosine",
        DistanceMetric::L2 => "l2",
        DistanceMetric::InnerProduct => "inner_product",
    }
}

fn metric_from_db_str(s: &str) -> RagResult<DistanceMetric> {
    match s {
        "cosine" => Ok(DistanceMetric::Cosine),
        "l2" => Ok(DistanceMetric::L2),
        "inner_product" => Ok(DistanceMetric::InnerProduct),
        other => Err(RagError::Backend(Box::new(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("unknown distance_metric in rag_collections: {other}"),
        )))),
    }
}

fn index_method_db_str(method: IndexMethod) -> &'static str {
    match method {
        IndexMethod::Flat => "flat",
        IndexMethod::Hnsw => "hnsw",
        IndexMethod::Diskann => "diskann",
    }
}

fn index_method_from_db_str(s: &str) -> RagResult<IndexMethod> {
    match s {
        "flat" => Ok(IndexMethod::Flat),
        "hnsw" => Ok(IndexMethod::Hnsw),
        "diskann" => Ok(IndexMethod::Diskann),
        other => Err(RagError::Backend(Box::new(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("unknown index_method in rag_collections: {other}"),
        )))),
    }
}

/// The pgvector distance operator for a metric (used in `ORDER BY`/`SELECT`).
fn distance_operator(metric: DistanceMetric) -> &'static str {
    match metric {
        DistanceMetric::Cosine => "<=>",
        DistanceMetric::L2 => "<->",
        DistanceMetric::InnerProduct => "<#>",
    }
}

/// The pgvector operator class for a metric (used in `CREATE INDEX ... USING hnsw`).
fn metric_ops_class(metric: DistanceMetric) -> &'static str {
    match metric {
        DistanceMetric::Cosine => "vector_cosine_ops",
        DistanceMetric::L2 => "vector_l2_ops",
        DistanceMetric::InnerProduct => "vector_ip_ops",
    }
}

/// Convert a raw pgvector distance into this crate's higher-is-better score
/// convention (matching `InMemoryVectorStore`'s `score()`).
fn distance_to_score(metric: DistanceMetric, distance: f64) -> f32 {
    match metric {
        DistanceMetric::Cosine => (1.0 - distance) as f32,
        DistanceMetric::L2 | DistanceMetric::InnerProduct => (-distance) as f32,
    }
}

// ── SQL identifier/literal quoting for dynamic DDL ─────────────────────────
//
// `CREATE INDEX ... WHERE collection = <literal>` cannot bind `collection` as
// a query parameter (Postgres does not accept parameters in DDL statement
// text), so the value is interpolated after manual escaping. Both helpers are
// pure and unit-tested below; every DDL string in this module built from
// caller-controlled data goes through one of them.

fn quote_ident(s: &str) -> String {
    format!("\"{}\"", s.replace('"', "\"\""))
}

fn quote_literal(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''"))
}

/// A collection name may contain characters that are legal in a quoted
/// identifier but awkward in an auto-generated index name; this only affects
/// readability (the identifier is still always wrapped by [`quote_ident`]).
fn sanitize_for_ident(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

fn index_name(collection: &str) -> String {
    format!(
        "idx_rag_chunks_embedding_{}",
        sanitize_for_ident(collection)
    )
}

#[async_trait]
impl RagStore for PgVectorStore {
    fn name(&self) -> &str {
        &self.name
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities {
            full_text: true,
            hybrid: true,
            filtering: true,
            // `sparse_embedding`/`multi_vector` are stored (JSONB) but no
            // retrieval path scores them yet — see the module doc comment.
            sparse: false,
            late_interaction: false,
            // `Diskann` is never advertised: pgvector has no DiskANN index,
            // and `ensure_collection` transparently substitutes `Hnsw` for
            // it, so a caller relying on `capabilities()` sees only what is
            // actually implemented, not the substitution.
            index_methods: vec![IndexMethod::Flat, IndexMethod::Hnsw],
        }
    }

    async fn ensure_collection(&self, spec: &CollectionSpec) -> RagResult<()> {
        let existing: Option<CollectionRow> = sqlx::query_as(
            "SELECT embedding_dim, distance_metric, index_method, created_at, embedding_source, embedding_version \
             FROM rag_collections WHERE name = $1",
        )
        .bind(&spec.name)
        .fetch_optional(&self.pool)
        .await
        .map_err(pg_err)?;

        if let Some(row) = existing {
            if row.embedding_dim as u32 != spec.embedding_dim {
                return Err(RagError::CollectionAlreadyExists(spec.name.clone()));
            }
            // Idempotent: matching dimension is a no-op, mirroring
            // `InMemoryVectorStore::ensure_collection`. The index method/
            // metric of an existing collection is not retroactively changed.
            return Ok(());
        }

        sqlx::query(
            "INSERT INTO rag_collections (name, embedding_dim, distance_metric, index_method) \
             VALUES ($1, $2, $3, $4)",
        )
        .bind(&spec.name)
        .bind(spec.embedding_dim as i32)
        .bind(metric_db_str(spec.distance_metric))
        .bind(index_method_db_str(spec.index_method))
        .execute(&self.pool)
        .await
        .map_err(pg_err)?;

        self.build_index(spec).await
    }

    async fn drop_collection(&self, collection: &str) -> RagResult<()> {
        let drop_index_sql = format!(
            "DROP INDEX IF EXISTS {}",
            quote_ident(&index_name(collection))
        );
        sqlx::query(&drop_index_sql)
            .execute(&self.pool)
            .await
            .map_err(pg_err)?;

        let result = sqlx::query("DELETE FROM rag_collections WHERE name = $1")
            .bind(collection)
            .execute(&self.pool)
            .await
            .map_err(pg_err)?;

        if result.rows_affected() == 0 {
            return Err(RagError::CollectionNotFound(collection.to_string()));
        }
        Ok(())
    }

    async fn get_collection(&self, collection: &str) -> RagResult<Option<CollectionSpec>> {
        let row: Option<CollectionRow> = sqlx::query_as(
            "SELECT embedding_dim, distance_metric, index_method, created_at, embedding_source, embedding_version \
             FROM rag_collections WHERE name = $1",
        )
        .bind(collection)
        .fetch_optional(&self.pool)
        .await
        .map_err(pg_err)?;

        row.map(|r| collection_row_to_spec(collection, r))
            .transpose()
    }

    async fn upsert_document(
        &self,
        collection: &str,
        document: &DocumentRecord,
        chunks: &[ChunkRecord],
    ) -> RagResult<DocumentId> {
        let spec = self
            .get_collection(collection)
            .await?
            .ok_or_else(|| RagError::CollectionNotFound(collection.to_string()))?;

        for chunk in chunks {
            if chunk.embedding.len() as u32 != spec.embedding_dim {
                return Err(RagError::EmbeddingDimMismatch {
                    expected: spec.embedding_dim,
                    got: chunk.embedding.len() as u32,
                });
            }
            validate_chunk_side_vectors(chunk)?;
        }

        let mut tx = self.pool.begin().await.map_err(pg_err)?;

        let existing_id: Option<Uuid> = if let Some(ext) = &document.external_id {
            sqlx::query_scalar(
                "SELECT id FROM rag_documents WHERE collection = $1 AND external_id = $2",
            )
            .bind(collection)
            .bind(ext)
            .fetch_optional(&mut *tx)
            .await
            .map_err(pg_err)?
        } else {
            None
        };

        let keywords_json = serde_json::to_value(&document.keywords).map_err(json_err)?;

        let doc_id: Uuid = if let Some(id) = existing_id {
            sqlx::query(
                "UPDATE rag_documents SET title = $1, mime = $2, source_uri = $3, full_text = $4, \
                 keywords = $5, entities = $6, labels = $7, metadata = $8 WHERE id = $9",
            )
            .bind(&document.title)
            .bind(&document.mime)
            .bind(&document.source_uri)
            .bind(&document.full_text)
            .bind(&keywords_json)
            .bind(&document.entities)
            .bind(&document.labels)
            .bind(&document.metadata)
            .bind(id)
            .execute(&mut *tx)
            .await
            .map_err(pg_err)?;

            sqlx::query("DELETE FROM rag_chunks WHERE document_id = $1")
                .bind(id)
                .execute(&mut *tx)
                .await
                .map_err(pg_err)?;
            id
        } else {
            sqlx::query_scalar(
                "INSERT INTO rag_documents \
                 (collection, external_id, title, mime, source_uri, full_text, keywords, entities, labels, metadata) \
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10) RETURNING id",
            )
            .bind(collection)
            .bind(&document.external_id)
            .bind(&document.title)
            .bind(&document.mime)
            .bind(&document.source_uri)
            .bind(&document.full_text)
            .bind(&keywords_json)
            .bind(&document.entities)
            .bind(&document.labels)
            .bind(&document.metadata)
            .fetch_one(&mut *tx)
            .await
            .map_err(pg_err)?
        };

        for chunk in chunks {
            let embedding = PgVector::from(chunk.embedding.clone());
            let sparse_json = chunk
                .sparse_embedding
                .as_ref()
                .map(serde_json::to_value)
                .transpose()
                .map_err(json_err)?;
            let multi_vector_json = chunk
                .multi_vector
                .as_ref()
                .map(serde_json::to_value)
                .transpose()
                .map_err(json_err)?;

            sqlx::query(
                "INSERT INTO rag_chunks \
                 (document_id, collection, external_id, ordinal, content, embedding, sparse_embedding, multi_vector, chunk_metadata) \
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
            )
            .bind(doc_id)
            .bind(collection)
            .bind(&chunk.external_id)
            .bind(chunk.ordinal as i32)
            .bind(&chunk.content)
            .bind(embedding)
            .bind(&sparse_json)
            .bind(&multi_vector_json)
            .bind(&chunk.chunk_metadata)
            .execute(&mut *tx)
            .await
            .map_err(pg_err)?;
        }

        tx.commit().await.map_err(pg_err)?;
        Ok(DocumentId(doc_id.to_string()))
    }

    async fn delete_documents(&self, collection: &str, ids: &[DocumentId]) -> RagResult<u64> {
        // Unknown/malformed ids are ignored (trait contract), not errored —
        // matches `InMemoryVectorStore`, whose opaque-string ids never fail
        // to parse. A Postgres id is a real UUID primary key, so a caller
        // that hands back a non-UUID string (e.g. from a different backend)
        // simply matches nothing rather than aborting the whole call.
        let uuids: Vec<Uuid> = ids
            .iter()
            .filter_map(|id| Uuid::parse_str(&id.0).ok())
            .collect();
        if uuids.is_empty() {
            return Ok(0);
        }

        let result =
            sqlx::query("DELETE FROM rag_documents WHERE collection = $1 AND id = ANY($2)")
                .bind(collection)
                .bind(&uuids)
                .execute(&self.pool)
                .await
                .map_err(pg_err)?;
        Ok(result.rows_affected())
    }

    async fn delete_by_filter(&self, collection: &str, filter: &Filter) -> RagResult<u64> {
        filter.validate()?;

        let mut qb = QueryBuilder::<Postgres>::new(
            "WITH matched AS (SELECT DISTINCT d.id FROM rag_documents d \
             JOIN rag_chunks c ON c.document_id = d.id WHERE d.collection = ",
        );
        qb.push_bind(collection.to_string());
        qb.push(" AND (");
        push_filter_sql(&mut qb, filter)?;
        qb.push(")) DELETE FROM rag_documents WHERE id IN (SELECT id FROM matched)");

        let result = qb.build().execute(&self.pool).await.map_err(pg_err)?;
        Ok(result.rows_affected())
    }

    async fn retrieve(&self, collection: &str, query: &RetrieveQuery) -> RagResult<RetrieveOutput> {
        let start = Instant::now();
        if !matches!(
            query.mode,
            RetrieveMode::Vector | RetrieveMode::FullText | RetrieveMode::Hybrid
        ) {
            return Err(RagError::UnsupportedMode {
                backend: self.name.clone(),
                mode: query.mode.as_str().to_string(),
            });
        }

        let spec = self
            .get_collection(collection)
            .await?
            .ok_or_else(|| RagError::CollectionNotFound(collection.to_string()))?;
        query.validate(&spec)?;

        match query.mode {
            RetrieveMode::Vector => self.retrieve_vector(collection, &spec, query, start).await,
            RetrieveMode::FullText => self.retrieve_full_text(collection, query, start).await,
            RetrieveMode::Hybrid => self.retrieve_hybrid(collection, &spec, query, start).await,
            RetrieveMode::Sparse | RetrieveMode::LateInteraction => {
                unreachable!("guarded by the capability check above")
            }
        }
    }

    async fn collection_stats(&self, collection: &str) -> RagResult<CollectionStats> {
        self.get_collection(collection)
            .await?
            .ok_or_else(|| RagError::CollectionNotFound(collection.to_string()))?;

        let row: StatsRow = sqlx::query_as(
            "SELECT \
               (SELECT COUNT(*) FROM rag_documents WHERE collection = $1) AS documents, \
               (SELECT COUNT(*) FROM rag_chunks WHERE collection = $1) AS chunks, \
               (SELECT MAX(ingested_at) FROM rag_documents WHERE collection = $1) AS last_ingested_at",
        )
        .bind(collection)
        .fetch_one(&self.pool)
        .await
        .map_err(pg_err)?;

        Ok(CollectionStats {
            documents: row.documents as u64,
            chunks: row.chunks as u64,
            last_ingested_at: row.last_ingested_at.map(|t| t.timestamp()),
        })
    }

    async fn list_collections(
        &self,
        limit: u32,
        offset: u32,
    ) -> RagResult<(Vec<CollectionSpec>, u64)> {
        let total: i64 = sqlx::query_scalar("SELECT count(*) FROM rag_collections")
            .fetch_one(&self.pool)
            .await
            .map_err(pg_err)?;

        let rows = sqlx::query(
            "SELECT name, embedding_dim, distance_metric, index_method, created_at, embedding_source, embedding_version \
             FROM rag_collections ORDER BY created_at DESC, name DESC LIMIT $1 OFFSET $2",
        )
        .bind(limit as i64)
        .bind(offset as i64)
        .fetch_all(&self.pool)
        .await
        .map_err(pg_err)?;

        let specs = rows
            .into_iter()
            .map(|row| {
                let name: String = row.try_get("name").map_err(pg_err)?;
                let collection_row = CollectionRow::from_row(&row).map_err(pg_err)?;
                collection_row_to_spec(&name, collection_row)
            })
            .collect::<RagResult<Vec<_>>>()?;
        Ok((specs, total as u64))
    }

    async fn list_documents(
        &self,
        collection: &str,
        limit: u32,
        offset: u32,
    ) -> RagResult<(Vec<DocumentSummary>, u64)> {
        if self.get_collection(collection).await?.is_none() {
            return Err(RagError::CollectionNotFound(collection.to_string()));
        }

        let total: i64 =
            sqlx::query_scalar("SELECT count(*) FROM rag_documents WHERE collection = $1")
                .bind(collection)
                .fetch_one(&self.pool)
                .await
                .map_err(pg_err)?;

        let rows = sqlx::query(
            "SELECT id, external_id, title, mime, keywords, labels, entities, metadata, ingested_at \
             FROM rag_documents WHERE collection = $1 \
             ORDER BY ingested_at DESC, id DESC LIMIT $2 OFFSET $3",
        )
        .bind(collection)
        .bind(limit as i64)
        .bind(offset as i64)
        .fetch_all(&self.pool)
        .await
        .map_err(pg_err)?;

        let summaries = rows
            .into_iter()
            .map(|row| document_row_to_summary(&row))
            .collect::<RagResult<Vec<_>>>()?;
        Ok((summaries, total as u64))
    }

    async fn set_embedding_provenance(
        &self,
        collection: &str,
        source: &str,
        version: u32,
    ) -> RagResult<()> {
        let result = sqlx::query(
            "UPDATE rag_collections SET embedding_source = $1, embedding_version = $2 WHERE name = $3",
        )
        .bind(source)
        .bind(version as i32)
        .bind(collection)
        .execute(&self.pool)
        .await
        .map_err(pg_err)?;

        if result.rows_affected() == 0 {
            return Err(RagError::CollectionNotFound(collection.to_string()));
        }
        Ok(())
    }

    async fn get_document_chunks(
        &self,
        collection: &str,
        document: &DocumentId,
    ) -> RagResult<Option<(DocumentRecord, Vec<ChunkRecord>)>> {
        if self.get_collection(collection).await?.is_none() {
            return Err(RagError::CollectionNotFound(collection.to_string()));
        }

        // Malformed/foreign ids simply match nothing, same tolerance as
        // `delete_documents`'s `filter_map(Uuid::parse_str(...).ok())`.
        let Ok(doc_uuid) = Uuid::parse_str(&document.0) else {
            return Ok(None);
        };

        let doc_row = sqlx::query(
            "SELECT external_id, title, mime, source_uri, full_text, keywords, entities, labels, metadata \
             FROM rag_documents WHERE id = $1 AND collection = $2",
        )
        .bind(doc_uuid)
        .bind(collection)
        .fetch_optional(&self.pool)
        .await
        .map_err(pg_err)?;

        let Some(doc_row) = doc_row else {
            return Ok(None);
        };

        let document_record = document_row_to_record(&doc_row)?;

        let chunk_rows = sqlx::query(
            "SELECT external_id, ordinal, content, embedding, sparse_embedding, multi_vector, chunk_metadata \
             FROM rag_chunks WHERE document_id = $1 ORDER BY ordinal",
        )
        .bind(doc_uuid)
        .fetch_all(&self.pool)
        .await
        .map_err(pg_err)?;

        let chunks = chunk_rows
            .iter()
            .map(chunk_row_to_record)
            .collect::<RagResult<Vec<_>>>()?;

        Ok(Some((document_record, chunks)))
    }

    async fn update_chunk_embeddings(
        &self,
        collection: &str,
        document: &DocumentId,
        embeddings: &[(u32, Vec<f32>)],
    ) -> RagResult<()> {
        let spec = self
            .get_collection(collection)
            .await?
            .ok_or_else(|| RagError::CollectionNotFound(collection.to_string()))?;

        for (_, vector) in embeddings {
            if vector.len() as u32 != spec.embedding_dim {
                return Err(RagError::EmbeddingDimMismatch {
                    expected: spec.embedding_dim,
                    got: vector.len() as u32,
                });
            }
        }

        // Malformed/foreign ids simply match nothing, same tolerance as
        // `delete_documents`'s `filter_map(Uuid::parse_str(...).ok())`.
        let Ok(doc_uuid) = Uuid::parse_str(&document.0) else {
            return Ok(());
        };

        for (ordinal, vector) in embeddings {
            let embedding = PgVector::from(vector.clone());
            sqlx::query(
                "UPDATE rag_chunks SET embedding = $1 \
                 WHERE document_id = $2 AND collection = $3 AND ordinal = $4",
            )
            .bind(embedding)
            .bind(doc_uuid)
            .bind(collection)
            .bind(*ordinal as i32)
            .execute(&self.pool)
            .await
            .map_err(pg_err)?;
        }
        Ok(())
    }
}

impl PgVectorStore {
    /// Create/refresh the ANN index for a collection. A no-op for
    /// [`IndexMethod::Flat`] (brute-force scan, no index needed).
    async fn build_index(&self, spec: &CollectionSpec) -> RagResult<()> {
        let mut effective_method = spec.index_method;
        if matches!(effective_method, IndexMethod::Diskann) {
            tracing::warn!(
                collection = %spec.name,
                "IndexMethod::Diskann has no pgvector equivalent (pgvector ships HNSW and \
                 IVFFlat, not DiskANN); substituting HNSW"
            );
            effective_method = IndexMethod::Hnsw;
        }

        if matches!(effective_method, IndexMethod::Flat) {
            return Ok(());
        }

        let ops = metric_ops_class(spec.distance_metric);
        // `embedding` is an unconstrained `vector` column (rows across
        // collections may have different dimensions), but pgvector's HNSW
        // builder rejects an index on an unconstrained column outright —
        // "column does not have dimensions" — even under a dimension-uniform
        // partial index (verified empirically against a live pgvector 0.8
        // instance; the migration's original comment assuming otherwise was
        // wrong). Casting to a fixed-dimension `vector(n)` inside the index
        // expression gives the index builder a concrete dimension without
        // constraining the underlying column, which every other collection's
        // partial index still shares.
        let sql = format!(
            "CREATE INDEX IF NOT EXISTS {} ON rag_chunks USING hnsw ((embedding::vector({})) {ops}) WHERE collection = {}",
            quote_ident(&index_name(&spec.name)),
            spec.embedding_dim,
            quote_literal(&spec.name),
        );
        sqlx::query(&sql)
            .execute(&self.pool)
            .await
            .map_err(pg_err)?;
        Ok(())
    }

    async fn retrieve_vector(
        &self,
        collection: &str,
        spec: &CollectionSpec,
        query: &RetrieveQuery,
        start: Instant,
    ) -> RagResult<RetrieveOutput> {
        let Some(query_vector) = &query.query_vector else {
            return Err(RagError::InvalidQuery(
                "pgvector backend cannot embed text; supply query_vector".to_string(),
            ));
        };
        let op = distance_operator(spec.distance_metric);
        let vector = PgVector::from(query_vector.clone());

        // Cast to a fixed-dimension `vector(n)` to match `build_index`'s
        // indexed expression exactly — Postgres only uses an expression
        // index when the query's `ORDER BY` expression is syntactically
        // identical to it. Without this cast the query is still correct
        // (an uncast `<op>` against `embedding` returns the same rows) but
        // silently falls back to a sequential scan, defeating the ANN index
        // entirely (verified empirically: `EXPLAIN` showed a seq scan until
        // this cast was added).
        let embedding_cast = format!("c.embedding::vector({})", spec.embedding_dim);

        let mut qb = QueryBuilder::<Postgres>::new("SELECT ");
        qb.push(CHUNK_DOC_COLUMNS);
        qb.push(", (");
        qb.push(&embedding_cast);
        qb.push(" ");
        qb.push(op);
        qb.push(" ");
        qb.push_bind(vector.clone());
        qb.push(") AS rank_value FROM rag_chunks c JOIN rag_documents d ON d.id = c.document_id WHERE c.collection = ");
        qb.push_bind(collection.to_string());
        push_optional_filter(&mut qb, query)?;
        qb.push(" ORDER BY ");
        qb.push(&embedding_cast);
        qb.push(" ");
        qb.push(op);
        qb.push(" ");
        qb.push_bind(vector);
        qb.push(" LIMIT ");
        qb.push_bind(effective_limit(query));

        let rows = qb.build().fetch_all(&self.pool).await.map_err(pg_err)?;
        let mut chunks = Vec::with_capacity(rows.len());
        for row in &rows {
            let distance: f64 = row.try_get("rank_value").map_err(pg_err)?;
            let score = distance_to_score(spec.distance_metric, distance);
            chunks.push(RowFields::from_row(row)?.into_retrieved_chunk(
                query,
                score,
                PrimaryScore::Vector { score },
            ));
        }
        Ok(finalize(chunks, query, RetrieveMode::Vector, start))
    }

    async fn retrieve_full_text(
        &self,
        collection: &str,
        query: &RetrieveQuery,
        start: Instant,
    ) -> RagResult<RetrieveOutput> {
        let text = query
            .query_text
            .as_deref()
            .expect("validated: query_text present for full_text mode");

        let (rows, _) = self
            .fetch_fulltext_candidates(collection, text, query, effective_limit(query))
            .await?;
        let mut chunks = Vec::with_capacity(rows.len());
        for (row, rank) in rows {
            chunks.push(row.into_retrieved_chunk(
                query,
                rank,
                PrimaryScore::FullText { score: rank },
            ));
        }
        Ok(finalize(chunks, query, RetrieveMode::FullText, start))
    }

    async fn retrieve_hybrid(
        &self,
        collection: &str,
        spec: &CollectionSpec,
        query: &RetrieveQuery,
        start: Instant,
    ) -> RagResult<RetrieveOutput> {
        let Some(query_vector) = &query.query_vector else {
            return Err(RagError::InvalidQuery(
                "pgvector backend's hybrid mode requires query_vector in addition to query_text"
                    .to_string(),
            ));
        };
        let text = query
            .query_text
            .as_deref()
            .expect("validated: query_text present for hybrid mode");
        let pool_size = effective_limit(query);

        let vector_rows = self
            .fetch_vector_candidates(collection, spec, query_vector, query, pool_size)
            .await?;
        let (fulltext_rows, _) = self
            .fetch_fulltext_candidates(collection, text, query, pool_size)
            .await?;

        let mut fused: std::collections::HashMap<String, FusedEntry> =
            std::collections::HashMap::new();
        for (rank, (fields, vector_score)) in vector_rows.into_iter().enumerate() {
            let key = fields.id.to_string();
            let entry = fused.entry(key).or_insert_with(|| FusedEntry::new(fields));
            entry.vector_score = vector_score;
            entry.rrf += 1.0 / (RRF_K + (rank as f64 + 1.0));
        }
        for (rank, (fields, full_text_score)) in fulltext_rows.into_iter().enumerate() {
            let key = fields.id.to_string();
            let entry = fused.entry(key).or_insert_with(|| FusedEntry::new(fields));
            entry.full_text_score = full_text_score;
            entry.rrf += 1.0 / (RRF_K + (rank as f64 + 1.0));
        }

        let mut chunks: Vec<RetrievedChunk> = fused
            .into_values()
            .map(|e| e.into_retrieved_chunk(query))
            .collect();
        chunks.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        Ok(finalize(chunks, query, RetrieveMode::Hybrid, start))
    }

    async fn fetch_vector_candidates(
        &self,
        collection: &str,
        spec: &CollectionSpec,
        query_vector: &[f32],
        query: &RetrieveQuery,
        limit: i64,
    ) -> RagResult<Vec<(RowFields, f32)>> {
        let op = distance_operator(spec.distance_metric);
        let vector = PgVector::from(query_vector.to_vec());

        let mut qb = QueryBuilder::<Postgres>::new("SELECT ");
        qb.push(CHUNK_DOC_COLUMNS);
        qb.push(", (c.embedding ");
        qb.push(op);
        qb.push(" ");
        qb.push_bind(vector.clone());
        qb.push(") AS rank_value FROM rag_chunks c JOIN rag_documents d ON d.id = c.document_id WHERE c.collection = ");
        qb.push_bind(collection.to_string());
        push_optional_filter(&mut qb, query)?;
        qb.push(" ORDER BY c.embedding ");
        qb.push(op);
        qb.push(" ");
        qb.push_bind(vector);
        qb.push(" LIMIT ");
        qb.push_bind(limit);

        let rows = qb.build().fetch_all(&self.pool).await.map_err(pg_err)?;
        let mut out = Vec::with_capacity(rows.len());
        for row in &rows {
            let distance: f64 = row.try_get("rank_value").map_err(pg_err)?;
            let score = distance_to_score(spec.distance_metric, distance);
            out.push((RowFields::from_row(row)?, score));
        }
        Ok(out)
    }

    async fn fetch_fulltext_candidates(
        &self,
        collection: &str,
        text: &str,
        query: &RetrieveQuery,
        limit: i64,
    ) -> RagResult<(Vec<(RowFields, f32)>, ())> {
        let mut qb = QueryBuilder::<Postgres>::new("SELECT ");
        qb.push(CHUNK_DOC_COLUMNS);
        qb.push(", ts_rank(c.content_tsv, plainto_tsquery('english', ");
        qb.push_bind(text.to_string());
        qb.push(
            ")) AS rank_value FROM rag_chunks c JOIN rag_documents d ON d.id = c.document_id \
                 WHERE c.collection = ",
        );
        qb.push_bind(collection.to_string());
        qb.push(" AND c.content_tsv @@ plainto_tsquery('english', ");
        qb.push_bind(text.to_string());
        qb.push(")");
        push_optional_filter(&mut qb, query)?;
        qb.push(" ORDER BY rank_value DESC LIMIT ");
        qb.push_bind(limit);

        let rows = qb.build().fetch_all(&self.pool).await.map_err(pg_err)?;
        let mut out = Vec::with_capacity(rows.len());
        for row in &rows {
            let rank: f32 = row.try_get("rank_value").map_err(pg_err)?;
            out.push((RowFields::from_row(row)?, rank));
        }
        Ok((out, ()))
    }
}

fn push_optional_filter(
    qb: &mut QueryBuilder<'_, Postgres>,
    query: &RetrieveQuery,
) -> RagResult<()> {
    if let Some(filter) = &query.filter {
        qb.push(" AND (");
        push_filter_sql(qb, filter)?;
        qb.push(")");
    }
    Ok(())
}

/// Candidate-pool size for a query: `top_k * candidate_multiplier` (default
/// multiplier `1`), matching `RetrieveQuery::validate`'s bounds so this never
/// needs its own cap beyond what validation already enforces.
fn effective_limit(query: &RetrieveQuery) -> i64 {
    let multiplier = query.candidate_multiplier.unwrap_or(1);
    i64::from(query.top_k) * i64::from(multiplier)
}

fn finalize(
    mut chunks: Vec<RetrievedChunk>,
    query: &RetrieveQuery,
    mode: RetrieveMode,
    start: Instant,
) -> RetrieveOutput {
    if query.group_by_document {
        let mut seen = HashSet::new();
        chunks.retain(|c| seen.insert(c.document_id.clone()));
    }
    chunks.truncate(query.top_k as usize);
    RetrieveOutput {
        mode,
        chunks,
        primary_latency_ms: start.elapsed().as_millis() as u64,
    }
}

// ── row <-> domain conversions ─────────────────────────────────────────────

#[derive(sqlx::FromRow)]
struct CollectionRow {
    embedding_dim: i32,
    distance_metric: String,
    index_method: String,
    created_at: chrono::DateTime<chrono::Utc>,
    embedding_source: Option<String>,
    embedding_version: i32,
}

/// Build a [`CollectionSpec`] from a [`CollectionRow`] plus the name (not
/// itself a row column in every query that selects one — `ensure_collection`'s
/// existence check does not need it).
fn collection_row_to_spec(name: &str, row: CollectionRow) -> RagResult<CollectionSpec> {
    Ok(CollectionSpec {
        name: name.to_string(),
        embedding_dim: row.embedding_dim as u32,
        distance_metric: metric_from_db_str(&row.distance_metric)?,
        index_method: index_method_from_db_str(&row.index_method)?,
        created_at: Some(row.created_at.timestamp()),
        embedding_source: row.embedding_source,
        embedding_version: row.embedding_version as u32,
    })
}

/// Build a [`DocumentSummary`] from a `rag_documents` row selected by
/// [`PgVectorStore::list_documents`]. A free function (not a `FromRow` impl)
/// because `keywords` needs a fallible JSON decode into `Vec<String>`, which
/// `#[derive(FromRow)]` cannot express.
fn document_row_to_summary(row: &PgRow) -> RagResult<DocumentSummary> {
    let id: Uuid = row.try_get("id").map_err(pg_err)?;
    let keywords: serde_json::Value = row.try_get("keywords").map_err(pg_err)?;
    let ingested_at: Option<chrono::DateTime<chrono::Utc>> =
        row.try_get("ingested_at").map_err(pg_err)?;
    Ok(DocumentSummary {
        id: DocumentId(id.to_string()),
        external_id: row.try_get("external_id").map_err(pg_err)?,
        title: row.try_get("title").map_err(pg_err)?,
        mime: row.try_get("mime").map_err(pg_err)?,
        keywords: serde_json::from_value(keywords).unwrap_or_default(),
        labels: row.try_get("labels").map_err(pg_err)?,
        entities: row.try_get("entities").map_err(pg_err)?,
        metadata: row.try_get("metadata").map_err(pg_err)?,
        ingested_at: ingested_at.map(|t| t.timestamp()),
    })
}

/// Build a [`DocumentRecord`] from a `rag_documents` row selected by
/// [`PgVectorStore::get_document_chunks`]. A free function (not a `FromRow`
/// impl) for the same reason as [`document_row_to_summary`]: `keywords` needs
/// a fallible JSON decode into `Vec<String>`.
fn document_row_to_record(row: &PgRow) -> RagResult<DocumentRecord> {
    let keywords: serde_json::Value = row.try_get("keywords").map_err(pg_err)?;
    Ok(DocumentRecord {
        external_id: row.try_get("external_id").map_err(pg_err)?,
        title: row.try_get("title").map_err(pg_err)?,
        mime: row.try_get("mime").map_err(pg_err)?,
        source_uri: row.try_get("source_uri").map_err(pg_err)?,
        full_text: row.try_get("full_text").map_err(pg_err)?,
        keywords: serde_json::from_value(keywords).unwrap_or_default(),
        entities: row.try_get("entities").map_err(pg_err)?,
        labels: row.try_get("labels").map_err(pg_err)?,
        metadata: row.try_get("metadata").map_err(pg_err)?,
    })
}

/// Build a [`ChunkRecord`] from a `rag_chunks` row selected by
/// [`PgVectorStore::get_document_chunks`]. Inverse of `upsert_document`'s
/// JSONB serialization of `sparse_embedding`/`multi_vector`.
fn chunk_row_to_record(row: &PgRow) -> RagResult<ChunkRecord> {
    let embedding: PgVector = row.try_get("embedding").map_err(pg_err)?;
    let ordinal: i32 = row.try_get("ordinal").map_err(pg_err)?;
    let sparse_json: Option<serde_json::Value> = row.try_get("sparse_embedding").map_err(pg_err)?;
    let multi_vector_json: Option<serde_json::Value> =
        row.try_get("multi_vector").map_err(pg_err)?;
    Ok(ChunkRecord {
        external_id: row.try_get("external_id").map_err(pg_err)?,
        ordinal: ordinal as u32,
        content: row.try_get("content").map_err(pg_err)?,
        embedding: embedding.to_vec(),
        sparse_embedding: sparse_json
            .map(serde_json::from_value)
            .transpose()
            .map_err(json_err)?,
        multi_vector: multi_vector_json
            .map(serde_json::from_value)
            .transpose()
            .map_err(json_err)?,
        chunk_metadata: row.try_get("chunk_metadata").map_err(pg_err)?,
    })
}

#[derive(sqlx::FromRow)]
struct StatsRow {
    documents: i64,
    chunks: i64,
    last_ingested_at: Option<chrono::DateTime<chrono::Utc>>,
}

/// The chunk + parent-document fields common to every retrieval mode,
/// extracted once from a [`PgRow`] and reused to build a [`RetrievedChunk`]
/// regardless of which mode produced the row.
struct RowFields {
    id: Uuid,
    document_id: Uuid,
    ordinal: i32,
    external_id: Option<String>,
    content: String,
    chunk_metadata: serde_json::Value,
    doc_external_id: Option<String>,
    doc_title: Option<String>,
    doc_mime: Option<String>,
    doc_keywords: serde_json::Value,
    doc_labels: serde_json::Value,
    doc_entities: serde_json::Value,
    doc_metadata: serde_json::Value,
    doc_ingested_at: Option<chrono::DateTime<chrono::Utc>>,
}

impl RowFields {
    fn from_row(row: &PgRow) -> RagResult<Self> {
        Ok(Self {
            id: row.try_get("id").map_err(pg_err)?,
            document_id: row.try_get("document_id").map_err(pg_err)?,
            ordinal: row.try_get("ordinal").map_err(pg_err)?,
            external_id: row.try_get("external_id").map_err(pg_err)?,
            content: row.try_get("content").map_err(pg_err)?,
            chunk_metadata: row.try_get("chunk_metadata").map_err(pg_err)?,
            doc_external_id: row.try_get("doc_external_id").map_err(pg_err)?,
            doc_title: row.try_get("doc_title").map_err(pg_err)?,
            doc_mime: row.try_get("doc_mime").map_err(pg_err)?,
            doc_keywords: row.try_get("doc_keywords").map_err(pg_err)?,
            doc_labels: row.try_get("doc_labels").map_err(pg_err)?,
            doc_entities: row.try_get("doc_entities").map_err(pg_err)?,
            doc_metadata: row.try_get("doc_metadata").map_err(pg_err)?,
            doc_ingested_at: row.try_get("doc_ingested_at").map_err(pg_err)?,
        })
    }

    fn into_retrieved_chunk(
        self,
        query: &RetrieveQuery,
        score: f32,
        primary_score: PrimaryScore,
    ) -> RetrievedChunk {
        let document_id = DocumentId(self.document_id.to_string());
        RetrievedChunk {
            id: ChunkId(self.id.to_string()),
            document_id: document_id.clone(),
            ordinal: self.ordinal as u32,
            external_id: self.external_id,
            content: query.include_content.then_some(self.content),
            score,
            primary_score,
            chunk_metadata: self.chunk_metadata,
            document: query.include_document.then(|| DocumentSummary {
                id: document_id,
                external_id: self.doc_external_id,
                title: self.doc_title,
                mime: self.doc_mime,
                keywords: serde_json::from_value(self.doc_keywords).unwrap_or_default(),
                labels: self.doc_labels,
                entities: self.doc_entities,
                metadata: self.doc_metadata,
                ingested_at: self.doc_ingested_at.map(|t| t.timestamp()),
            }),
        }
    }
}

/// Accumulator for hybrid retrieval's reciprocal rank fusion.
struct FusedEntry {
    fields: RowFields,
    vector_score: f32,
    full_text_score: f32,
    rrf: f64,
}

impl FusedEntry {
    fn new(fields: RowFields) -> Self {
        Self {
            fields,
            vector_score: 0.0,
            full_text_score: 0.0,
            rrf: 0.0,
        }
    }

    fn into_retrieved_chunk(self, query: &RetrieveQuery) -> RetrievedChunk {
        let rrf = self.rrf as f32;
        let primary_score = PrimaryScore::Hybrid {
            vector: self.vector_score,
            full_text: self.full_text_score,
            sparse: 0.0,
            rrf,
        };
        self.fields.into_retrieved_chunk(query, rrf, primary_score)
    }
}

// ── filter -> SQL translation ──────────────────────────────────────────────
//
// Field references are resolved through `FilterField::parse`, which already
// validates the field is one of a fixed whitelist (`filter.rs`), so the SQL
// fragments below are built from a `match` over known strings — never from
// unvalidated caller input. `metadata.`/`chunk_metadata.` path segments are
// the one caller-controlled piece and are always passed as a bound `TEXT[]`
// parameter to `#>`/`#>>`, never interpolated into SQL text.

fn push_filter_sql(qb: &mut QueryBuilder<'_, Postgres>, filter: &Filter) -> RagResult<()> {
    match filter {
        Filter::Eq { field, value } => {
            let parsed = field.parse()?;
            push_field_jsonb_expr(qb, &parsed);
            qb.push(" = ");
            qb.push_bind(value.clone());
        }
        Filter::In { field, values } => {
            if values.is_empty() {
                qb.push("false");
                return Ok(());
            }
            qb.push("(");
            for (i, value) in values.iter().enumerate() {
                if i > 0 {
                    qb.push(" OR ");
                }
                let parsed = field.parse()?;
                push_field_jsonb_expr(qb, &parsed);
                qb.push(" = ");
                qb.push_bind(value.clone());
            }
            qb.push(")");
        }
        Filter::ArrayContains { field, value } => {
            let parsed = field.parse()?;
            push_field_jsonb_expr(qb, &parsed);
            qb.push(" @> jsonb_build_array(");
            qb.push_bind(value.clone());
            qb.push(")");
        }
        Filter::Range {
            field,
            gte,
            gt,
            lte,
            lt,
        } => {
            let parsed = field.parse()?;
            let bounds: [(&str, &Option<serde_json::Value>); 4] =
                [(">=", gte), (">", gt), ("<=", lte), ("<", lt)];
            let mut wrote_any = false;
            qb.push("(");
            for (op, bound) in bounds {
                let Some(value) = bound else { continue };
                if wrote_any {
                    qb.push(" AND ");
                }
                push_field_jsonb_expr(qb, &parsed);
                qb.push(" ");
                qb.push(op);
                qb.push(" ");
                qb.push_bind(value.clone());
                wrote_any = true;
            }
            if !wrote_any {
                qb.push("true");
            }
            qb.push(")");
        }
        Filter::TextMatch { field, query } => {
            let parsed = field.parse()?;
            push_field_text_expr(qb, &parsed);
            qb.push(" ILIKE ");
            qb.push_bind(format!("%{}%", escape_like(query)));
            qb.push(" ESCAPE '\\'");
        }
        Filter::And { filters } => {
            if filters.is_empty() {
                qb.push("true");
                return Ok(());
            }
            qb.push("(");
            for (i, f) in filters.iter().enumerate() {
                if i > 0 {
                    qb.push(" AND ");
                }
                push_filter_sql(qb, f)?;
            }
            qb.push(")");
        }
        Filter::Or { filters } => {
            if filters.is_empty() {
                qb.push("false");
                return Ok(());
            }
            qb.push("(");
            for (i, f) in filters.iter().enumerate() {
                if i > 0 {
                    qb.push(" OR ");
                }
                push_filter_sql(qb, f)?;
            }
            qb.push(")");
        }
        Filter::Not { filter } => {
            qb.push("NOT (");
            push_filter_sql(qb, filter)?;
            qb.push(")");
        }
    }
    Ok(())
}

fn path_segments(dotted: &str) -> Vec<String> {
    dotted.split('.').map(str::to_string).collect()
}

/// Escape `%`/`_`/`\` so `ILIKE '%<escaped>%'` matches `query` as a literal
/// substring, matching `InMemoryVectorStore`'s `str::contains` semantics
/// (which has no wildcard concept at all).
fn escape_like(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_")
}

/// Push a SQL expression yielding `jsonb`, for structural equality/range/
/// containment comparisons (mirrors `resolve_field` in `backends/memory.rs`).
fn push_field_jsonb_expr(qb: &mut QueryBuilder<'_, Postgres>, parsed: &ParsedField) {
    match parsed.namespace {
        FilterNamespace::Doc => match parsed.path.as_str() {
            "full_text" => qb.push("to_jsonb(d.full_text)"),
            "title" => qb.push("to_jsonb(d.title)"),
            "mime" => qb.push("to_jsonb(d.mime)"),
            "external_id" => qb.push("to_jsonb(d.external_id)"),
            "source_uri" => qb.push("to_jsonb(d.source_uri)"),
            "keywords" => qb.push("d.keywords"),
            "labels" => qb.push("d.labels"),
            "entities" => qb.push("d.entities"),
            "ingested_at" => qb.push("to_jsonb(extract(epoch from d.ingested_at)::bigint)"),
            path if path.starts_with("metadata.") => {
                qb.push("(d.metadata #> ");
                qb.push_bind(path_segments(&path["metadata.".len()..]));
                qb.push(")")
            }
            // Unreachable: `FilterField::parse` already rejects any doc.*
            // path not in this list before `push_filter_sql` is ever called.
            _ => qb.push("to_jsonb(NULL::text)"),
        },
        FilterNamespace::Chunk => match parsed.path.as_str() {
            "ordinal" => qb.push("to_jsonb(c.ordinal)"),
            "external_id" => qb.push("to_jsonb(c.external_id)"),
            "content" => qb.push("to_jsonb(c.content)"),
            path if path.starts_with("chunk_metadata.") => {
                qb.push("(c.chunk_metadata #> ");
                qb.push_bind(path_segments(&path["chunk_metadata.".len()..]));
                qb.push(")")
            }
            _ => qb.push("to_jsonb(NULL::text)"),
        },
    };
}

/// Push a SQL expression yielding `text`, for `ILIKE` substring matching.
fn push_field_text_expr(qb: &mut QueryBuilder<'_, Postgres>, parsed: &ParsedField) {
    match parsed.namespace {
        FilterNamespace::Doc => match parsed.path.as_str() {
            "full_text" => qb.push("d.full_text"),
            "title" => qb.push("d.title"),
            "mime" => qb.push("d.mime"),
            "external_id" => qb.push("d.external_id"),
            "source_uri" => qb.push("d.source_uri"),
            "keywords" => qb.push("d.keywords::text"),
            "labels" => qb.push("d.labels::text"),
            "entities" => qb.push("d.entities::text"),
            "ingested_at" => qb.push("d.ingested_at::text"),
            path if path.starts_with("metadata.") => {
                qb.push("(d.metadata #>> ");
                qb.push_bind(path_segments(&path["metadata.".len()..]));
                qb.push(")")
            }
            _ => qb.push("NULL::text"),
        },
        FilterNamespace::Chunk => match parsed.path.as_str() {
            "ordinal" => qb.push("c.ordinal::text"),
            "external_id" => qb.push("c.external_id"),
            "content" => qb.push("c.content"),
            path if path.starts_with("chunk_metadata.") => {
                qb.push("(c.chunk_metadata #>> ");
                qb.push_bind(path_segments(&path["chunk_metadata.".len()..]));
                qb.push(")")
            }
            _ => qb.push("NULL::text"),
        },
    };
}

#[cfg(test)]
mod pure_tests {
    use super::*;

    #[test]
    fn should_double_quotes_when_quoting_identifier() {
        assert_eq!(quote_ident(r#"foo"bar"#), r#""foo""bar""#);
    }

    #[test]
    fn should_double_single_quotes_when_quoting_literal() {
        assert_eq!(quote_literal("foo'bar"), "'foo''bar'");
    }

    #[test]
    fn should_replace_non_alnum_when_sanitizing_for_ident() {
        assert_eq!(sanitize_for_ident("my-collection.v2"), "my_collection_v2");
    }

    #[test]
    fn should_escape_wildcards_when_escaping_like_query() {
        assert_eq!(escape_like("50%_off\\sale"), "50\\%\\_off\\\\sale");
    }

    #[test]
    fn should_split_on_dots_when_building_path_segments() {
        assert_eq!(path_segments("foo.bar.baz"), vec!["foo", "bar", "baz"]);
    }

    #[test]
    fn should_round_trip_when_converting_distance_metric_to_and_from_db_str() {
        for metric in [
            DistanceMetric::Cosine,
            DistanceMetric::L2,
            DistanceMetric::InnerProduct,
        ] {
            let s = metric_db_str(metric);
            assert_eq!(metric_from_db_str(s).unwrap(), metric);
        }
    }

    #[test]
    fn should_round_trip_when_converting_index_method_to_and_from_db_str() {
        for method in [IndexMethod::Flat, IndexMethod::Hnsw, IndexMethod::Diskann] {
            let s = index_method_db_str(method);
            assert_eq!(index_method_from_db_str(s).unwrap(), method);
        }
    }

    #[tokio::test]
    async fn should_never_advertise_diskann_in_capabilities() {
        // A caller relying on `Capabilities::index_methods` to decide whether
        // to request `IndexMethod::Diskann` must see it is unsupported here —
        // the substitution to Hnsw happens silently otherwise (with only a
        // `tracing::warn!`), so this is the capability-negotiation contract's
        // enforcement point.
        let pool = sqlx::Pool::<Postgres>::connect_lazy("postgres://localhost/does-not-matter")
            .expect("connect_lazy never touches the network");
        let store = PgVectorStore::new(pool);
        assert!(!store
            .capabilities()
            .index_methods
            .contains(&IndexMethod::Diskann));
        assert!(store
            .capabilities()
            .index_methods
            .contains(&IndexMethod::Hnsw));
    }
}

#[cfg(test)]
mod live_tests {
    use super::*;
    use crate::filter::FilterField;
    use crate::types::{MultiVector, SparseVector};

    // These tests are ignored by default because they require a running
    // Postgres with the `pgvector` extension available (see
    // `hacienda-core/src/store/postgres/connection.rs`'s `connect_and_migrate`
    // for the pattern this mirrors). Run with:
    //   DATABASE_URL=postgres://... cargo test -p hacienda-rag --features postgres \
    //     --lib backends::pgvector::live_tests -- --ignored --test-threads=1
    //
    // `--test-threads=1`: every test shares one Postgres instance and uses
    // collection names scoped by a random suffix to avoid cross-test
    // collisions, but the partial-index DDL in `ensure_collection` is not
    // itself transactional with the rest of a test, so serializing avoids any
    // window for one test's `DROP INDEX`/`CREATE INDEX` to race another's.

    async fn test_pool() -> PgPool {
        let database_url =
            std::env::var("DATABASE_URL").expect("DATABASE_URL must be set for integration tests");
        let pool = PgVectorStore::connect(&database_url)
            .await
            .expect("connect failed");
        PgVectorStore::migrate(&pool).await.expect("migrate failed");
        pool
    }

    fn unique_collection(prefix: &str) -> String {
        format!("{prefix}-{}", Uuid::new_v4())
    }

    fn chunk(ordinal: u32, content: &str, embedding: Vec<f32>) -> ChunkRecord {
        ChunkRecord {
            external_id: None,
            ordinal,
            content: content.to_string(),
            embedding,
            sparse_embedding: None,
            multi_vector: None,
            chunk_metadata: serde_json::Value::Null,
        }
    }

    #[tokio::test]
    #[ignore]
    async fn should_create_and_fetch_collection_when_ensure_collection_succeeds() {
        let store = PgVectorStore::new(test_pool().await);
        let name = unique_collection("coll");
        store
            .ensure_collection(&CollectionSpec::new(&name, 3))
            .await
            .unwrap();

        let spec = store.get_collection(&name).await.unwrap();
        assert_eq!(spec.map(|s| s.embedding_dim), Some(3));
    }

    #[tokio::test]
    #[ignore]
    async fn should_be_idempotent_when_ensure_collection_repeats_matching_dim() {
        let store = PgVectorStore::new(test_pool().await);
        let name = unique_collection("coll");
        store
            .ensure_collection(&CollectionSpec::new(&name, 4))
            .await
            .unwrap();
        store
            .ensure_collection(&CollectionSpec::new(&name, 4))
            .await
            .unwrap();

        let spec = store.get_collection(&name).await.unwrap();
        assert_eq!(spec.map(|s| s.embedding_dim), Some(4));
    }

    #[tokio::test]
    #[ignore]
    async fn should_reject_ensure_collection_when_dim_mismatches_existing() {
        let store = PgVectorStore::new(test_pool().await);
        let name = unique_collection("coll");
        store
            .ensure_collection(&CollectionSpec::new(&name, 4))
            .await
            .unwrap();

        let err = store
            .ensure_collection(&CollectionSpec::new(&name, 8))
            .await
            .unwrap_err();
        assert!(matches!(err, RagError::CollectionAlreadyExists(ref n) if n == &name));
    }

    #[tokio::test]
    #[ignore]
    async fn should_remove_collection_and_cascade_when_dropped() {
        let store = PgVectorStore::new(test_pool().await);
        let name = unique_collection("coll");
        store
            .ensure_collection(&CollectionSpec::new(&name, 2))
            .await
            .unwrap();
        store
            .upsert_document(
                &name,
                &DocumentRecord::default(),
                &[chunk(0, "a", vec![1.0, 0.0])],
            )
            .await
            .unwrap();

        store.drop_collection(&name).await.unwrap();
        assert!(store.get_collection(&name).await.unwrap().is_none());
    }

    #[tokio::test]
    #[ignore]
    async fn should_reject_drop_when_collection_is_unknown() {
        let store = PgVectorStore::new(test_pool().await);
        let err = store
            .drop_collection(&unique_collection("missing"))
            .await
            .unwrap_err();
        assert!(matches!(err, RagError::CollectionNotFound(_)));
    }

    #[tokio::test]
    #[ignore]
    async fn should_warn_and_substitute_hnsw_when_index_method_is_diskann() {
        let store = PgVectorStore::new(test_pool().await);
        let name = unique_collection("coll");
        // Not asserting on the `tracing::warn!` output itself (no subscriber
        // is installed in this test); the observable contract under test is
        // that Diskann does not error and the collection ends up usable for
        // vector retrieval exactly as an Hnsw collection would.
        let mut spec = CollectionSpec::new(&name, 2);
        spec.index_method = IndexMethod::Diskann;
        store.ensure_collection(&spec).await.unwrap();

        store
            .upsert_document(
                &name,
                &DocumentRecord::default(),
                &[chunk(0, "a", vec![1.0, 0.0])],
            )
            .await
            .unwrap();
        let q = RetrieveQuery {
            query_vector: Some(vec![1.0, 0.0]),
            ..RetrieveQuery::vector(5)
        };
        let out = store.retrieve(&name, &q).await.unwrap();
        assert_eq!(out.chunks.len(), 1);
    }

    #[tokio::test]
    #[ignore]
    async fn should_order_by_similarity_when_retrieving_after_upsert() {
        let store = PgVectorStore::new(test_pool().await);
        let name = unique_collection("coll");
        store
            .ensure_collection(&CollectionSpec::new(&name, 3))
            .await
            .unwrap();

        let doc = DocumentRecord {
            full_text: "hello world".to_string(),
            ..Default::default()
        };
        let chunks = vec![
            chunk(0, "near", vec![1.0, 0.0, 0.0]),
            chunk(1, "far", vec![0.0, 1.0, 0.0]),
        ];
        store.upsert_document(&name, &doc, &chunks).await.unwrap();

        let q = RetrieveQuery {
            query_vector: Some(vec![1.0, 0.0, 0.0]),
            include_content: true,
            ..RetrieveQuery::vector(10)
        };
        let out = store.retrieve(&name, &q).await.unwrap();
        assert_eq!(out.chunks.len(), 2);
        assert_eq!(out.chunks[0].content.as_deref(), Some("near"));
    }

    #[tokio::test]
    #[ignore]
    async fn should_reject_upsert_when_embedding_dimension_mismatches() {
        let store = PgVectorStore::new(test_pool().await);
        let name = unique_collection("coll");
        store
            .ensure_collection(&CollectionSpec::new(&name, 3))
            .await
            .unwrap();

        let bad = vec![chunk(0, "x", vec![1.0, 0.0])];
        let err = store
            .upsert_document(&name, &DocumentRecord::default(), &bad)
            .await
            .unwrap_err();
        assert!(matches!(
            err,
            RagError::EmbeddingDimMismatch {
                expected: 3,
                got: 2
            }
        ));
    }

    #[tokio::test]
    #[ignore]
    async fn should_replace_chunks_when_external_id_upsert_repeats() {
        let store = PgVectorStore::new(test_pool().await);
        let name = unique_collection("coll");
        store
            .ensure_collection(&CollectionSpec::new(&name, 2))
            .await
            .unwrap();

        let doc = DocumentRecord {
            external_id: Some("ext-1".to_string()),
            ..Default::default()
        };
        store
            .upsert_document(&name, &doc, &[chunk(0, "v1", vec![1.0, 0.0])])
            .await
            .unwrap();
        store
            .upsert_document(&name, &doc, &[chunk(0, "v2", vec![0.0, 1.0])])
            .await
            .unwrap();

        let stats = store.collection_stats(&name).await.unwrap();
        assert_eq!(stats.documents, 1);
        assert_eq!(stats.chunks, 1);
    }

    #[tokio::test]
    #[ignore]
    async fn should_remove_by_id_when_deleting_documents() {
        let store = PgVectorStore::new(test_pool().await);
        let name = unique_collection("coll");
        store
            .ensure_collection(&CollectionSpec::new(&name, 2))
            .await
            .unwrap();

        let id = store
            .upsert_document(
                &name,
                &DocumentRecord::default(),
                &[chunk(0, "a", vec![1.0, 0.0])],
            )
            .await
            .unwrap();

        let removed = store
            .delete_documents(&name, std::slice::from_ref(&id))
            .await
            .unwrap();
        assert_eq!(removed, 1);
        let stats = store.collection_stats(&name).await.unwrap();
        assert_eq!(stats.documents, 0);
        assert_eq!(stats.chunks, 0);
    }

    #[tokio::test]
    #[ignore]
    async fn should_ignore_unknown_ids_when_deleting_documents() {
        let store = PgVectorStore::new(test_pool().await);
        let name = unique_collection("coll");
        store
            .ensure_collection(&CollectionSpec::new(&name, 2))
            .await
            .unwrap();

        let removed = store
            .delete_documents(&name, &[DocumentId("does-not-exist".to_string())])
            .await
            .unwrap();
        assert_eq!(removed, 0);
    }

    #[tokio::test]
    #[ignore]
    async fn should_remove_matching_documents_when_deleting_by_filter() {
        let store = PgVectorStore::new(test_pool().await);
        let name = unique_collection("coll");
        store
            .ensure_collection(&CollectionSpec::new(&name, 2))
            .await
            .unwrap();

        let keep = DocumentRecord {
            title: Some("keep".to_string()),
            ..Default::default()
        };
        let drop = DocumentRecord {
            title: Some("drop".to_string()),
            ..Default::default()
        };
        store
            .upsert_document(&name, &keep, &[chunk(0, "a", vec![1.0, 0.0])])
            .await
            .unwrap();
        store
            .upsert_document(&name, &drop, &[chunk(0, "b", vec![0.0, 1.0])])
            .await
            .unwrap();

        let removed = store
            .delete_by_filter(
                &name,
                &Filter::Eq {
                    field: FilterField("doc.title".to_string()),
                    value: serde_json::json!("drop"),
                },
            )
            .await
            .unwrap();
        assert_eq!(removed, 1);
        let stats = store.collection_stats(&name).await.unwrap();
        assert_eq!(stats.documents, 1);
    }

    #[tokio::test]
    #[ignore]
    async fn should_constrain_results_when_filter_is_applied() {
        let store = PgVectorStore::new(test_pool().await);
        let name = unique_collection("coll");
        store
            .ensure_collection(&CollectionSpec::new(&name, 2))
            .await
            .unwrap();

        let a = DocumentRecord {
            title: Some("keep".to_string()),
            full_text: "a".to_string(),
            ..Default::default()
        };
        let b = DocumentRecord {
            title: Some("drop".to_string()),
            full_text: "b".to_string(),
            ..Default::default()
        };
        store
            .upsert_document(&name, &a, &[chunk(0, "a", vec![1.0, 0.0])])
            .await
            .unwrap();
        store
            .upsert_document(&name, &b, &[chunk(0, "b", vec![1.0, 0.0])])
            .await
            .unwrap();

        let q = RetrieveQuery {
            query_vector: Some(vec![1.0, 0.0]),
            filter: Some(Filter::Eq {
                field: FilterField("doc.title".to_string()),
                value: serde_json::json!("keep"),
            }),
            ..RetrieveQuery::vector(10)
        };
        let out = store.retrieve(&name, &q).await.unwrap();
        assert_eq!(out.chunks.len(), 1);
    }

    #[tokio::test]
    #[ignore]
    async fn should_rank_by_relevance_when_retrieving_full_text() {
        let store = PgVectorStore::new(test_pool().await);
        let name = unique_collection("coll");
        store
            .ensure_collection(&CollectionSpec::new(&name, 2))
            .await
            .unwrap();

        let doc = DocumentRecord::default();
        let chunks = vec![
            chunk(0, "the quick brown fox jumps", vec![1.0, 0.0]),
            chunk(1, "an entirely unrelated sentence", vec![0.0, 1.0]),
        ];
        store.upsert_document(&name, &doc, &chunks).await.unwrap();

        let q = RetrieveQuery {
            mode: RetrieveMode::FullText,
            query_text: Some("quick fox".to_string()),
            include_content: true,
            ..RetrieveQuery::vector(10)
        };
        let out = store.retrieve(&name, &q).await.unwrap();
        assert_eq!(out.chunks.len(), 1, "only the matching chunk should rank");
        assert_eq!(
            out.chunks[0].content.as_deref(),
            Some("the quick brown fox jumps")
        );
    }

    #[tokio::test]
    #[ignore]
    async fn should_fuse_vector_and_full_text_when_retrieving_hybrid() {
        let store = PgVectorStore::new(test_pool().await);
        let name = unique_collection("coll");
        store
            .ensure_collection(&CollectionSpec::new(&name, 2))
            .await
            .unwrap();

        let doc = DocumentRecord::default();
        let chunks = vec![
            chunk(0, "quick fox", vec![1.0, 0.0]),
            chunk(1, "slow turtle", vec![0.0, 1.0]),
        ];
        store.upsert_document(&name, &doc, &chunks).await.unwrap();

        let q = RetrieveQuery {
            mode: RetrieveMode::Hybrid,
            query_text: Some("quick fox".to_string()),
            query_vector: Some(vec![1.0, 0.0]),
            include_content: true,
            ..RetrieveQuery::vector(10)
        };
        let out = store.retrieve(&name, &q).await.unwrap();
        assert_eq!(out.mode, RetrieveMode::Hybrid);
        assert_eq!(out.chunks[0].content.as_deref(), Some("quick fox"));
        assert!(matches!(
            out.chunks[0].primary_score,
            PrimaryScore::Hybrid { .. }
        ));
    }

    #[tokio::test]
    #[ignore]
    async fn should_reject_retrieve_when_sparse_mode_is_unsupported() {
        let store = PgVectorStore::new(test_pool().await);
        let name = unique_collection("coll");
        store
            .ensure_collection(&CollectionSpec::new(&name, 2))
            .await
            .unwrap();

        let q = RetrieveQuery {
            mode: RetrieveMode::Sparse,
            query_sparse: Some(SparseVector {
                indices: vec![1],
                values: vec![1.0],
            }),
            ..RetrieveQuery::vector(5)
        };
        let err = store.retrieve(&name, &q).await.unwrap_err();
        assert!(matches!(err, RagError::UnsupportedMode { ref mode, .. } if mode == "sparse"));
    }

    #[tokio::test]
    #[ignore]
    async fn should_reject_retrieve_when_late_interaction_mode_is_unsupported() {
        let store = PgVectorStore::new(test_pool().await);
        let name = unique_collection("coll");
        store
            .ensure_collection(&CollectionSpec::new(&name, 2))
            .await
            .unwrap();

        let q = RetrieveQuery {
            mode: RetrieveMode::LateInteraction,
            query_text: Some("hi".to_string()),
            query_vector: Some(vec![1.0, 0.0]),
            query_multi_vector: Some(MultiVector {
                num_tokens: 1,
                dim: 2,
                data: vec![1.0, 0.0],
            }),
            ..RetrieveQuery::vector(5)
        };
        let err = store.retrieve(&name, &q).await.unwrap_err();
        assert!(
            matches!(err, RagError::UnsupportedMode { ref mode, .. } if mode == "late_interaction")
        );
    }

    #[tokio::test]
    #[ignore]
    async fn should_report_aggregate_stats_when_collection_has_documents() {
        let store = PgVectorStore::new(test_pool().await);
        let name = unique_collection("coll");
        store
            .ensure_collection(&CollectionSpec::new(&name, 2))
            .await
            .unwrap();

        store
            .upsert_document(
                &name,
                &DocumentRecord::default(),
                &[chunk(0, "a", vec![1.0, 0.0])],
            )
            .await
            .unwrap();
        store
            .upsert_document(
                &name,
                &DocumentRecord::default(),
                &[chunk(0, "b", vec![0.0, 1.0]), chunk(1, "c", vec![1.0, 1.0])],
            )
            .await
            .unwrap();

        let stats = store.collection_stats(&name).await.unwrap();
        assert_eq!(stats.documents, 2);
        assert_eq!(stats.chunks, 3);
    }

    #[tokio::test]
    #[ignore]
    async fn should_reject_collection_stats_when_collection_is_unknown() {
        let store = PgVectorStore::new(test_pool().await);
        let err = store
            .collection_stats(&unique_collection("missing"))
            .await
            .unwrap_err();
        assert!(matches!(err, RagError::CollectionNotFound(_)));
    }

    /// Regression test for the `_sqlx_migrations` version-1 collision: before
    /// hacienda-rag's migrations were renumbered `0100+` (see
    /// `hacienda-core/migrations/README.md`), both this crate and
    /// hacienda-core shipped a migration numbered `1`, and running both
    /// crates' migrators against one shared database produced
    /// `VersionMismatch(1)` because sqlx's `_sqlx_migrations` table is one
    /// per physical database, not one per crate. This proves both migrators
    /// now coexist on one database.
    #[tokio::test]
    #[ignore]
    async fn should_run_both_crates_migrations_against_one_shared_database() {
        let database_url =
            std::env::var("DATABASE_URL").expect("DATABASE_URL must be set for integration tests");
        let pool = PgVectorStore::connect(&database_url)
            .await
            .expect("connect failed");

        PgVectorStore::migrate(&pool)
            .await
            .expect("hacienda-rag migrate failed");
        hacienda_core::store::postgres::connection::migrate(&pool)
            .await
            .expect("hacienda-core migrate failed");

        let versions: Vec<i64> =
            sqlx::query_scalar("SELECT version FROM _sqlx_migrations ORDER BY version")
                .fetch_all(&pool)
                .await
                .expect("failed to read _sqlx_migrations");

        assert!(
            versions.iter().any(|v| *v < 100),
            "expected at least one hacienda-core migration (<100) in _sqlx_migrations, got {versions:?}"
        );
        assert!(
            versions.iter().any(|v| *v >= 100),
            "expected at least one hacienda-rag migration (>=100) in _sqlx_migrations, got {versions:?}"
        );
    }
}
