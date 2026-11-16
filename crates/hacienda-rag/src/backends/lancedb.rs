//! LanceDB-backed [`RagStore`](crate::RagStore) implementation.
//!
//! The adapter intentionally uses only the stable, official LanceDB 0.17 API:
//! Arrow schemas/record batches, table merge-insert, table deletion and query
//! execution.  A collection is represented by a durable metadata row and two
//! tables (`documents` and `chunks`).  Splitting the tables means an empty
//! document remains representable and keeps document metadata from being
//! duplicated into every chunk.
//!
//! Filtering is evaluated from the crate's allow-listed [`Filter`] IR rather
//! than interpolating caller strings into Lance SQL.  This keeps the store
//! boundary safe for callers that scope one store instance to one tenant.

use crate::capability::Capabilities;
use crate::error::{RagError, RagResult};
use crate::filter::{Filter, FilterField, FilterNamespace};
use crate::query::{RetrieveMode, RetrieveOutput, RetrieveQuery};
use crate::store::RagStore;
use crate::types::{
    validate_chunk_side_vectors, ChunkId, ChunkRecord, CollectionSpec, CollectionStats,
    DistanceMetric, DocumentId, DocumentRecord, DocumentSummary, IndexMethod, PrimaryScore,
    RetrievedChunk,
};
use arrow_array::{
    Array, ArrayRef, FixedSizeListArray, Float32Array, Int64Array, RecordBatch,
    RecordBatchIterator, StringArray, UInt32Array,
};
use arrow_schema::{DataType, Field, Schema, SchemaRef};
use async_trait::async_trait;
use futures::TryStreamExt;
use lancedb::index::scalar::FullTextSearchQuery;
use lancedb::index::{Index, IndexType};
use lancedb::query::ExecutableQuery;
use lancedb::query::QueryBase;
use lancedb::{Connection, Table};
use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering as AtomicOrdering};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use uuid::Uuid;

const META_TABLE: &str = "__hacienda_rag_collections";
const PREFIX: &str = "__hacienda_rag_";
const FTS_INDEX_NAME: &str = "__hacienda_rag_chunks_content_fts";
const RRF_K: f64 = 60.0;
#[derive(Clone)]
struct StoredDocument {
    id: DocumentId,
    record: DocumentRecord,
    ingested_at: i64,
}

#[derive(Clone)]
struct StoredChunk {
    id: ChunkId,
    document_id: DocumentId,
    record: ChunkRecord,
}

/// A durable, single-tenant LanceDB RAG store.
pub struct LanceDbStore {
    name: String,
    db: Connection,
    fts_ready: Arc<AtomicBool>,
    fts_lock: Arc<futures::lock::Mutex<()>>,
    write_lock: Arc<futures::lock::Mutex<()>>,
}

impl LanceDbStore {
    /// Connect to a local or remote LanceDB URI.
    pub async fn connect(uri: impl AsRef<str>) -> RagResult<Self> {
        let db = lancedb::connect(uri.as_ref())
            .execute()
            .await
            .map_err(backend_err)?;
        let store = Self {
            name: format!("lancedb:{}", uri.as_ref()),
            db,
            fts_ready: Arc::new(AtomicBool::new(false)),
            fts_lock: Arc::new(futures::lock::Mutex::new(())),
            write_lock: Arc::new(futures::lock::Mutex::new(())),
        };
        store.refresh_fts_status().await?;
        Ok(store)
    }

    /// Connect with a stable display name, useful when several stores share a process.
    pub async fn connect_with_name(
        name: impl Into<String>,
        uri: impl AsRef<str>,
    ) -> RagResult<Self> {
        let mut store = Self::connect(uri).await?;
        store.name = name.into();
        Ok(store)
    }

    fn table_name(collection: &str, suffix: &str) -> String {
        // Hex encoding makes arbitrary caller collection names safe table names.
        let encoded = collection
            .as_bytes()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>();
        format!("{PREFIX}{encoded}_{suffix}")
    }

    async fn table_names(&self) -> RagResult<Vec<String>> {
        self.db.table_names().execute().await.map_err(backend_err)
    }

    async fn has_table(&self, name: &str) -> RagResult<bool> {
        Ok(self.table_names().await?.iter().any(|n| n == name))
    }

    async fn open_table(&self, name: &str) -> RagResult<Table> {
        self.db
            .open_table(name)
            .execute()
            .await
            .map_err(backend_err)
    }

    async fn refresh_fts_status(&self) -> RagResult<()> {
        let chunk_tables = self
            .table_names()
            .await?
            .into_iter()
            .filter(|name| name.starts_with(PREFIX) && name.ends_with("_chunks"))
            .collect::<Vec<_>>();
        let mut ready = !chunk_tables.is_empty();
        for name in chunk_tables {
            let table = self.open_table(&name).await?;
            ready &= has_fts_index(&table).await;
        }
        self.fts_ready.store(ready, AtomicOrdering::Release);
        Ok(())
    }

    async fn ensure_fts_index(&self, table: &Table, rebuild: bool) -> RagResult<()> {
        let _guard = self.fts_lock.lock().await;
        let existing = table
            .list_indices()
            .await
            .map_err(backend_err)?
            .into_iter()
            .find(is_content_fts_index);
        if existing.is_none() || rebuild {
            let mut builder = table
                .create_index(&["content"], Index::FTS(Default::default()))
                .replace(true);
            if let Some(index) = existing {
                builder = builder.name(index.name);
            } else {
                builder = builder.name(FTS_INDEX_NAME.to_string());
            }
            builder.execute().await.map_err(backend_err)?;
        }
        drop(_guard);
        self.refresh_fts_status().await
    }

    async fn bm25_candidates(
        &self,
        table: &Table,
        query: &str,
        limit: usize,
    ) -> RagResult<HashMap<String, f32>> {
        let fts_query = FullTextSearchQuery::new(query.to_string())
            .with_columns(&["content".to_string()])
            .map_err(backend_err)?
            .limit(Some(limit as i64));
        let batches = table
            .query()
            .full_text_search(fts_query)
            .limit(limit)
            .execute()
            .await
            .map_err(backend_err)?
            .try_collect::<Vec<_>>()
            .await
            .map_err(backend_err)?;
        let mut scores = HashMap::new();
        for batch in batches {
            let ids = column(&batch, "chunk_id")?
                .as_any()
                .downcast_ref::<StringArray>()
                .ok_or_else(|| RagError::InvalidQuery("LanceDB FTS chunk_id is not Utf8".into()))?;
            let values = column(&batch, "_score")?
                .as_any()
                .downcast_ref::<Float32Array>()
                .ok_or_else(|| {
                    RagError::InvalidQuery("LanceDB FTS _score is not Float32".into())
                })?;
            for row in 0..batch.num_rows() {
                scores.insert(ids.value(row).to_string(), values.value(row));
            }
        }
        Ok(scores)
    }

    async fn ensure_metadata_table(&self) -> RagResult<Table> {
        if self.has_table(META_TABLE).await? {
            return self.open_table(META_TABLE).await;
        }
        let table = self
            .db
            .create_empty_table(META_TABLE, metadata_schema())
            .execute()
            .await
            .map_err(backend_err)?;
        Ok(table)
    }

    async fn collection_tables(&self, collection: &str) -> RagResult<(Table, Table)> {
        let docs = self
            .open_table(&Self::table_name(collection, "documents"))
            .await?;
        let chunks = self
            .open_table(&Self::table_name(collection, "chunks"))
            .await?;
        Ok((docs, chunks))
    }

    async fn spec(&self, collection: &str) -> RagResult<Option<CollectionSpec>> {
        let Some(row) = self
            .read_metadata()
            .await?
            .into_iter()
            .find(|s| s.name == collection)
        else {
            return Ok(None);
        };
        Ok(Some(row))
    }

    async fn require_spec(&self, collection: &str) -> RagResult<CollectionSpec> {
        self.spec(collection)
            .await?
            .ok_or_else(|| RagError::CollectionNotFound(collection.to_string()))
    }

    async fn read_metadata(&self) -> RagResult<Vec<CollectionSpec>> {
        if !self.has_table(META_TABLE).await? {
            return Ok(Vec::new());
        }
        let table = self.open_table(META_TABLE).await?;
        let batches = table
            .query()
            .execute()
            .await
            .map_err(backend_err)?
            .try_collect::<Vec<_>>()
            .await
            .map_err(backend_err)?;
        let mut out = Vec::new();
        for batch in batches {
            for row in 0..batch.num_rows() {
                let name = string_at(&batch, "name", row)?;
                let dim = uint_at(&batch, "embedding_dim", row)?;
                let distance_metric = parse_metric(&string_at(&batch, "distance_metric", row)?)?;
                let index_method = parse_index(&string_at(&batch, "index_method", row)?)?;
                let created = int_at(&batch, "created_at", row)?;
                let source = empty_as_none(string_at(&batch, "embedding_source", row)?);
                out.push(CollectionSpec {
                    name,
                    embedding_dim: dim,
                    distance_metric,
                    index_method,
                    created_at: (created != 0).then_some(created),
                    embedding_source: source,
                    embedding_version: uint_at(&batch, "embedding_version", row)?,
                });
            }
        }
        out.sort_by(|a, b| {
            b.created_at
                .cmp(&a.created_at)
                .then_with(|| b.name.cmp(&a.name))
        });
        Ok(out)
    }

    async fn read_documents(&self, table: &Table) -> RagResult<Vec<StoredDocument>> {
        let batches = table
            .query()
            .execute()
            .await
            .map_err(backend_err)?
            .try_collect::<Vec<_>>()
            .await
            .map_err(backend_err)?;
        let mut docs = Vec::new();
        for batch in batches {
            for row in 0..batch.num_rows() {
                docs.push(StoredDocument {
                    id: DocumentId(string_at(&batch, "doc_id", row)?),
                    record: DocumentRecord {
                        external_id: empty_as_none(string_at(&batch, "external_id", row)?),
                        title: empty_as_none(string_at(&batch, "title", row)?),
                        mime: empty_as_none(string_at(&batch, "mime", row)?),
                        source_uri: empty_as_none(string_at(&batch, "source_uri", row)?),
                        full_text: string_at(&batch, "full_text", row)?,
                        keywords: json_at(&batch, "keywords", row)?,
                        entities: json_value_at(&batch, "entities", row)?,
                        labels: json_value_at(&batch, "labels", row)?,
                        metadata: json_value_at(&batch, "metadata", row)?,
                        dossier_id: empty_as_none(string_at(&batch, "dossier_id", row)?),
                        version_ref: empty_as_none(string_at(&batch, "version_ref", row)?),
                    },
                    ingested_at: int_at(&batch, "ingested_at", row)?,
                });
            }
        }
        Ok(docs)
    }

    async fn read_chunks(&self, table: &Table) -> RagResult<Vec<StoredChunk>> {
        let batches = table
            .query()
            .execute()
            .await
            .map_err(backend_err)?
            .try_collect::<Vec<_>>()
            .await
            .map_err(backend_err)?;
        let mut chunks = Vec::new();
        for batch in batches {
            for row in 0..batch.num_rows() {
                chunks.push(StoredChunk {
                    id: ChunkId(string_at(&batch, "chunk_id", row)?),
                    document_id: DocumentId(string_at(&batch, "doc_id", row)?),
                    record: ChunkRecord {
                        external_id: empty_as_none(string_at(&batch, "external_id", row)?),
                        ordinal: uint_at(&batch, "ordinal", row)?,
                        content: string_at(&batch, "content", row)?,
                        embedding: vector_at(&batch, "embedding", row)?,
                        sparse_embedding: json_at(&batch, "sparse_embedding", row)?,
                        multi_vector: json_at(&batch, "multi_vector", row)?,
                        chunk_metadata: json_value_at(&batch, "chunk_metadata", row)?,
                        dossier_id: empty_as_none(string_at(&batch, "dossier_id", row)?),
                        version_ref: empty_as_none(string_at(&batch, "version_ref", row)?),
                    },
                });
            }
        }
        Ok(chunks)
    }

    async fn read_collection_rows(
        &self,
        collection: &str,
    ) -> RagResult<(CollectionSpec, Vec<StoredDocument>, Vec<StoredChunk>)> {
        let spec = self.require_spec(collection).await?;
        let (docs, chunks) = self.collection_tables(collection).await?;
        Ok((
            spec,
            self.read_documents(&docs).await?,
            self.read_chunks(&chunks).await?,
        ))
    }

    async fn delete_doc_rows(&self, collection: &str, ids: &HashSet<String>) -> RagResult<u64> {
        if ids.is_empty() {
            return Ok(0);
        }
        let (docs, chunks) = self.collection_tables(collection).await?;
        let existing = self.read_documents(&docs).await?;
        let count = existing.iter().filter(|d| ids.contains(&d.id.0)).count() as u64;
        for id in ids {
            let escaped = sql_quote(id);
            chunks
                .delete(&format!("doc_id = '{escaped}'"))
                .await
                .map_err(backend_err)?;
            docs.delete(&format!("doc_id = '{escaped}'"))
                .await
                .map_err(backend_err)?;
        }
        Ok(count)
    }

    async fn merge_metadata(&self, spec: &CollectionSpec) -> RagResult<()> {
        let table = self.ensure_metadata_table().await?;
        let batch = metadata_batch(spec)?;
        let mut merge = table.merge_insert(&["name"]);
        merge
            .when_matched_update_all(None)
            .when_not_matched_insert_all();
        merge
            .execute(Box::new(one_batch(batch, metadata_schema())))
            .await
            .map(|_| ())
            .map_err(backend_err)
    }

    async fn merge_document(
        &self,
        table: &Table,
        id: &DocumentId,
        doc: &DocumentRecord,
        ingested_at: i64,
    ) -> RagResult<()> {
        let batch = document_batch(id, doc, ingested_at)?;
        let mut merge = table.merge_insert(&["doc_id"]);
        merge
            .when_matched_update_all(None)
            .when_not_matched_insert_all();
        merge
            .execute(Box::new(one_batch(batch, document_schema())))
            .await
            .map(|_| ())
            .map_err(backend_err)
    }

    async fn replace_chunks(
        &self,
        table: &Table,
        document: &DocumentId,
        rows: &[StoredChunk],
        dim: u32,
    ) -> RagResult<()> {
        if rows.is_empty() {
            return table
                .delete(&format!("doc_id = '{}'", sql_quote(&document.0)))
                .await
                .map(|_| ())
                .map_err(backend_err);
        }
        let batch = chunk_batch(rows, dim)?;
        let mut merge = table.merge_insert(&["chunk_id"]);
        merge
            .when_matched_update_all(None)
            .when_not_matched_insert_all()
            .when_not_matched_by_source_delete(Some(format!(
                "doc_id = '{}'",
                sql_quote(&document.0)
            )));
        merge
            .execute(Box::new(one_batch(batch, chunk_schema(dim))))
            .await
            .map(|_| ())
            .map_err(backend_err)
    }
}

#[async_trait]
impl RagStore for LanceDbStore {
    fn name(&self) -> &str {
        &self.name
    }

    fn capabilities(&self) -> Capabilities {
        let fts_ready = self.fts_ready.load(AtomicOrdering::Acquire);
        Capabilities {
            full_text: fts_ready,
            hybrid: fts_ready,
            filtering: true,
            sparse: false,
            late_interaction: false,
            index_methods: vec![IndexMethod::Flat],
        }
    }

    async fn ensure_collection(&self, spec: &CollectionSpec) -> RagResult<()> {
        let _write = self.write_lock.lock().await;
        if let Some(existing) = self.spec(&spec.name).await? {
            if existing.embedding_dim != spec.embedding_dim
                || existing.distance_metric != spec.distance_metric
            {
                return Err(RagError::CollectionAlreadyExists(spec.name.clone()));
            }
            let chunks = self
                .open_table(&Self::table_name(&spec.name, "chunks"))
                .await?;
            self.ensure_fts_index(&chunks, false).await?;
            return Ok(());
        }
        let docs_name = Self::table_name(&spec.name, "documents");
        let chunks_name = Self::table_name(&spec.name, "chunks");
        if !self.has_table(&docs_name).await? {
            self.db
                .create_empty_table(&docs_name, document_schema())
                .execute()
                .await
                .map_err(backend_err)?;
        }
        if !self.has_table(&chunks_name).await? {
            self.db
                .create_empty_table(&chunks_name, chunk_schema(spec.embedding_dim))
                .execute()
                .await
                .map_err(backend_err)?;
        }
        // A previous attempt may have created only one table. Refuse a
        // mismatched schema rather than publishing unusable collection metadata.
        let docs = self.open_table(&docs_name).await?;
        let chunks = self.open_table(&chunks_name).await?;
        if docs.schema().await.map_err(backend_err)?.as_ref() != document_schema().as_ref()
            || chunks.schema().await.map_err(backend_err)?.as_ref()
                != chunk_schema(spec.embedding_dim).as_ref()
        {
            return Err(RagError::InvalidQuery(
                "incompatible partial LanceDB collection".into(),
            ));
        }
        self.ensure_fts_index(&chunks, false).await?;
        let mut persisted = spec.clone();
        persisted.created_at = Some(now_secs());
        self.merge_metadata(&persisted).await
    }

    async fn drop_collection(&self, collection: &str) -> RagResult<()> {
        let _write = self.write_lock.lock().await;
        self.require_spec(collection).await?;
        for suffix in ["documents", "chunks"] {
            let name = Self::table_name(collection, suffix);
            if self.has_table(&name).await? {
                self.db.drop_table(name, &[]).await.map_err(backend_err)?;
            }
        }
        let meta = self.ensure_metadata_table().await?;
        meta.delete(&format!("name = '{}'", sql_quote(collection)))
            .await
            .map_err(backend_err)?;
        self.refresh_fts_status().await
    }

    async fn get_collection(&self, collection: &str) -> RagResult<Option<CollectionSpec>> {
        self.spec(collection).await
    }

    async fn upsert_document(
        &self,
        collection: &str,
        document: &DocumentRecord,
        chunks: &[ChunkRecord],
    ) -> RagResult<DocumentId> {
        let _write = self.write_lock.lock().await;
        let spec = self.require_spec(collection).await?;
        for chunk in chunks {
            validate_chunk_side_vectors(chunk)?;
            if chunk.embedding.len() as u32 != spec.embedding_dim {
                return Err(RagError::EmbeddingDimMismatch {
                    expected: spec.embedding_dim,
                    got: chunk.embedding.len() as u32,
                });
            }
        }
        let (docs, chunk_table) = self.collection_tables(collection).await?;
        let existing = self.read_documents(&docs).await?;
        let id = document
            .external_id
            .as_ref()
            .and_then(|external| {
                existing
                    .iter()
                    .find(|d| d.record.external_id.as_ref() == Some(external))
                    .map(|d| d.id.clone())
            })
            .unwrap_or_else(|| {
                document.external_id.as_ref().map_or_else(
                    || DocumentId(format!("doc-{}", Uuid::new_v4())),
                    |external| {
                        let mut key = collection.as_bytes().to_vec();
                        key.push(0);
                        key.extend_from_slice(external.as_bytes());
                        DocumentId(format!(
                            "doc-ext-{}",
                            Uuid::new_v5(&Uuid::NAMESPACE_OID, &key)
                        ))
                    },
                )
            });
        // Le document est la source autoritative du scoping juridique : les
        // chunks produits par le chunking côté API (dossier_id / version_ref
        // à None) héritent du scope de leur document parent à l'upsert, sinon
        // une requête dossier/version retournerait zéro résultat malgré un
        // document correctement scopé.
        let rows = chunks
            .iter()
            .map(|record| {
                let mut scoped = record.clone();
                if scoped.dossier_id.is_none() {
                    scoped.dossier_id = document.dossier_id.clone();
                }
                if scoped.version_ref.is_none() {
                    scoped.version_ref = document.version_ref.clone();
                }
                StoredChunk {
                    id: ChunkId(format!("{}:{}", id.0, scoped.ordinal)),
                    document_id: id.clone(),
                    record: scoped,
                }
            })
            .collect::<Vec<_>>();
        self.replace_chunks(&chunk_table, &id, &rows, spec.embedding_dim)
            .await?;
        self.merge_document(&docs, &id, document, now_secs())
            .await?;
        self.ensure_fts_index(&chunk_table, true).await?;
        Ok(id)
    }

    async fn delete_documents(&self, collection: &str, ids: &[DocumentId]) -> RagResult<u64> {
        let _write = self.write_lock.lock().await;
        self.require_spec(collection).await?;
        let count = self
            .delete_doc_rows(collection, &ids.iter().map(|id| id.0.clone()).collect())
            .await?;
        let chunks = self
            .open_table(&Self::table_name(collection, "chunks"))
            .await?;
        self.ensure_fts_index(&chunks, true).await?;
        Ok(count)
    }

    async fn delete_by_filter(&self, collection: &str, filter: &Filter) -> RagResult<u64> {
        let _write = self.write_lock.lock().await;
        filter.validate()?;
        let (_, docs, chunks) = self.read_collection_rows(collection).await?;
        let ids = docs
            .iter()
            .filter(|doc| {
                chunks
                    .iter()
                    .filter(|chunk| chunk.document_id == doc.id)
                    .any(|chunk| eval_filter(filter, doc, &chunk.record))
            })
            .map(|d| d.id.0.clone())
            .collect::<HashSet<_>>();
        let count = self.delete_doc_rows(collection, &ids).await?;
        let table = self
            .open_table(&Self::table_name(collection, "chunks"))
            .await?;
        self.ensure_fts_index(&table, true).await?;
        Ok(count)
    }

    async fn retrieve(&self, collection: &str, query: &RetrieveQuery) -> RagResult<RetrieveOutput> {
        let (spec, docs, chunks) = self.read_collection_rows(collection).await?;
        query.validate(&spec)?;
        if query.mode == RetrieveMode::Hybrid && query.query_vector.is_none() {
            return Err(RagError::InvalidQuery(
                "LanceDB hybrid mode requires query_vector in addition to query_text".to_string(),
            ));
        }
        if matches!(
            query.mode,
            RetrieveMode::Sparse | RetrieveMode::LateInteraction
        ) {
            return Err(RagError::UnsupportedMode {
                backend: self.name.clone(),
                mode: query.mode.as_str().to_string(),
            });
        }
        let native_text = matches!(query.mode, RetrieveMode::FullText | RetrieveMode::Hybrid);
        let fts_scores = if native_text {
            let chunk_table = self
                .open_table(&Self::table_name(collection, "chunks"))
                .await?;
            self.ensure_fts_index(&chunk_table, false).await?;
            let candidate_limit = if query.mode == RetrieveMode::Hybrid {
                query.top_k as usize * query.candidate_multiplier.unwrap_or(1) as usize
            } else {
                chunks.len().max(1)
            };
            self.bm25_candidates(
                &chunk_table,
                query.query_text.as_deref().expect("validated text query"),
                candidate_limit,
            )
            .await?
        } else {
            HashMap::new()
        };
        let query = query.clone();
        tokio::task::spawn_blocking(move || {
            let started = std::time::Instant::now();
            let docs_by_id = docs
                .into_iter()
                .map(|d| (d.id.0.clone(), d))
                .collect::<HashMap<_, _>>();
            let mut scored = Vec::new();
            for chunk in chunks {
                let Some(doc) = docs_by_id.get(&chunk.document_id.0) else {
                    continue;
                };
                // Hard retrieval boundary: a document_id-scoped query never
                // surfaces a sibling document's chunks, whatever mode, filter,
                // or top_k.
                if let Some(document_id) = &query.document_id {
                    if &chunk.document_id != document_id {
                        continue;
                    }
                }
                // Hard dossier/version boundary: never surface chunks from a
                // sibling dossier or version, whatever the mode, filter, or
                // top_k. Chunks carry these fields denormalized.
                if let Some(dossier_id) = &query.dossier_id {
                    if chunk.record.dossier_id.as_deref() != Some(dossier_id.as_str()) {
                        continue;
                    }
                }
                if let Some(version_ref) = &query.version_ref {
                    if chunk.record.version_ref.as_deref() != Some(version_ref.as_str()) {
                        continue;
                    }
                }
                if query
                    .filter
                    .as_ref()
                    .is_some_and(|filter| !eval_filter(filter, doc, &chunk.record))
                {
                    continue;
                }
                let vector_score = query
                    .query_vector
                    .as_ref()
                    .map(|v| dense_score(spec.distance_metric, v, &chunk.record.embedding));
                let text_score = if native_text {
                    fts_scores.get(&chunk.id.0).copied().unwrap_or(0.0)
                } else {
                    query
                        .query_text
                        .as_ref()
                        .map(|q| text_score(q, &chunk.record.content))
                        .unwrap_or(0.0)
                };
                if query.mode == RetrieveMode::FullText && text_score == 0.0 {
                    continue;
                }
                let (score, primary) = match query.mode {
                    RetrieveMode::Vector => (
                        vector_score.unwrap_or(text_score),
                        PrimaryScore::Vector {
                            score: vector_score.unwrap_or(text_score),
                        },
                    ),
                    RetrieveMode::FullText => {
                        (text_score, PrimaryScore::FullText { score: text_score })
                    }
                    RetrieveMode::Hybrid => {
                        let vector = vector_score.unwrap_or(0.0);
                        (
                            0.0,
                            PrimaryScore::Hybrid {
                                vector,
                                full_text: text_score,
                                sparse: 0.0,
                                rrf: 0.0,
                            },
                        )
                    }
                    RetrieveMode::Sparse | RetrieveMode::LateInteraction => unreachable!(),
                };
                scored.push((score, chunk, doc.id.0.clone(), primary));
            }
            if query.mode == RetrieveMode::Hybrid {
                let candidate_limit =
                    query.top_k as usize * query.candidate_multiplier.unwrap_or(1) as usize;
                let mut vector_order = (0..scored.len()).collect::<Vec<_>>();
                vector_order.sort_by(|a, b| {
                    let score = |index: usize| match scored[index].3 {
                        PrimaryScore::Hybrid { vector, .. } => vector,
                        _ => unreachable!("hybrid query only produces hybrid score components"),
                    };
                    score(*b).partial_cmp(&score(*a)).unwrap_or(Ordering::Equal)
                });
                vector_order.truncate(candidate_limit);

                let mut text_order = (0..scored.len())
                    .filter(|index| match scored[*index].3 {
                        PrimaryScore::Hybrid { full_text, .. } => full_text > 0.0,
                        _ => unreachable!("hybrid query only produces hybrid score components"),
                    })
                    .collect::<Vec<_>>();
                text_order.sort_by(|a, b| {
                    let score = |index: usize| match scored[index].3 {
                        PrimaryScore::Hybrid { full_text, .. } => full_text,
                        _ => unreachable!("hybrid query only produces hybrid score components"),
                    };
                    score(*b).partial_cmp(&score(*a)).unwrap_or(Ordering::Equal)
                });
                text_order.truncate(candidate_limit);

                let mut fused = vec![0.0_f64; scored.len()];
                let mut included = vec![false; scored.len()];
                for (rank, index) in vector_order.into_iter().enumerate() {
                    fused[index] += 1.0 / (RRF_K + rank as f64 + 1.0);
                    included[index] = true;
                }
                for (rank, index) in text_order.into_iter().enumerate() {
                    fused[index] += 1.0 / (RRF_K + rank as f64 + 1.0);
                    included[index] = true;
                }
                for (index, entry) in scored.iter_mut().enumerate() {
                    let rrf = fused[index] as f32;
                    entry.0 = rrf;
                    if let PrimaryScore::Hybrid { rrf: score, .. } = &mut entry.3 {
                        *score = rrf;
                    }
                }
                scored = scored
                    .into_iter()
                    .enumerate()
                    .filter_map(|(index, entry)| included[index].then_some(entry))
                    .collect();
            }
            scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(Ordering::Equal));
            if query.group_by_document {
                let mut seen = HashSet::new();
                scored.retain(|(_, chunk, _, _)| seen.insert(chunk.document_id.0.clone()));
            }
            let chunks = scored
                .into_iter()
                .take(query.top_k as usize)
                .map(|(score, chunk, doc_id, primary)| RetrievedChunk {
                    id: chunk.id,
                    document_id: chunk.document_id,
                    ordinal: chunk.record.ordinal,
                    external_id: chunk.record.external_id,
                    content: query.include_content.then_some(chunk.record.content),
                    score,
                    primary_score: primary,
                    chunk_metadata: chunk.record.chunk_metadata,
                    document: query
                        .include_document
                        .then(|| summarize(&docs_by_id[&doc_id])),
                })
                .collect();
            Ok(RetrieveOutput {
                mode: query.mode,
                chunks,
                primary_latency_ms: started.elapsed().as_millis() as u64,
            })
        })
        .await
        .map_err(backend_err)?
    }

    async fn collection_stats(&self, collection: &str) -> RagResult<CollectionStats> {
        let (_, docs, chunks) = self.read_collection_rows(collection).await?;
        Ok(CollectionStats {
            documents: docs.len() as u64,
            chunks: chunks.len() as u64,
            last_ingested_at: docs.iter().map(|d| d.ingested_at).max().filter(|v| *v != 0),
        })
    }

    async fn list_collections(
        &self,
        limit: u32,
        offset: u32,
    ) -> RagResult<(Vec<CollectionSpec>, u64)> {
        let all = self.read_metadata().await?;
        let total = all.len() as u64;
        Ok((
            all.into_iter()
                .skip(offset as usize)
                .take(limit as usize)
                .collect(),
            total,
        ))
    }

    async fn list_documents(
        &self,
        collection: &str,
        limit: u32,
        offset: u32,
    ) -> RagResult<(Vec<DocumentSummary>, u64)> {
        let (_, mut docs, _) = self.read_collection_rows(collection).await?;
        docs.sort_by_key(|d| std::cmp::Reverse(d.ingested_at));
        let total = docs.len() as u64;
        Ok((
            docs.into_iter()
                .skip(offset as usize)
                .take(limit as usize)
                .map(|d| summarize(&d))
                .collect(),
            total,
        ))
    }

    async fn set_embedding_provenance(
        &self,
        collection: &str,
        source: &str,
        version: u32,
    ) -> RagResult<()> {
        let mut spec = self.require_spec(collection).await?;
        spec.embedding_source = Some(source.to_string());
        spec.embedding_version = version;
        self.merge_metadata(&spec).await
    }

    async fn get_document_chunks(
        &self,
        collection: &str,
        document: &DocumentId,
    ) -> RagResult<Option<(DocumentRecord, Vec<ChunkRecord>)>> {
        let (_, docs, mut chunks) = self.read_collection_rows(collection).await?;
        let Some(doc) = docs.into_iter().find(|d| d.id == *document) else {
            return Ok(None);
        };
        chunks.retain(|c| c.document_id == *document);
        chunks.sort_by_key(|c| c.record.ordinal);
        Ok(Some((
            doc.record,
            chunks.into_iter().map(|c| c.record).collect(),
        )))
    }

    async fn update_chunk_embeddings(
        &self,
        collection: &str,
        document: &DocumentId,
        embeddings: &[(u32, Vec<f32>)],
    ) -> RagResult<()> {
        let _write = self.write_lock.lock().await;
        let spec = self.require_spec(collection).await?;
        for (_, embedding) in embeddings {
            if embedding.len() as u32 != spec.embedding_dim {
                return Err(RagError::EmbeddingDimMismatch {
                    expected: spec.embedding_dim,
                    got: embedding.len() as u32,
                });
            }
            if embedding.iter().any(|value| !value.is_finite()) {
                return Err(RagError::InvalidQuery(
                    "embedding contains a non-finite value".into(),
                ));
            }
        }
        let table = self.collection_tables(collection).await?.1;
        for (ordinal, embedding) in embeddings {
            let expression = format!(
                "[{}]",
                embedding
                    .iter()
                    .map(|value| format!("{value:?}"))
                    .collect::<Vec<_>>()
                    .join(", ")
            );
            table
                .update()
                .only_if(format!(
                    "doc_id = '{}' AND ordinal = {ordinal}",
                    sql_quote(&document.0)
                ))
                .column("embedding", expression)
                .execute()
                .await
                .map_err(backend_err)?;
        }
        Ok(())
    }
}

fn backend_err<E: std::error::Error + Send + Sync + 'static>(e: E) -> RagError {
    RagError::Backend(Box::new(e))
}
async fn has_fts_index(table: &Table) -> bool {
    table
        .list_indices()
        .await
        .map(|indices| indices.iter().any(is_content_fts_index))
        .unwrap_or(false)
}
fn is_content_fts_index(index: &lancedb::index::IndexConfig) -> bool {
    index.index_type == IndexType::FTS && index.columns.len() == 1 && index.columns[0] == "content"
}
fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or_default()
}
fn sql_quote(value: &str) -> String {
    value.replace('\'', "''")
}
fn empty_as_none(value: String) -> Option<String> {
    (!value.is_empty()).then_some(value)
}
fn one_batch(
    batch: RecordBatch,
    schema: SchemaRef,
) -> RecordBatchIterator<std::vec::IntoIter<Result<RecordBatch, arrow_schema::ArrowError>>> {
    RecordBatchIterator::new(vec![Ok(batch)].into_iter(), schema)
}

fn metadata_schema() -> SchemaRef {
    Arc::new(Schema::new(vec![
        Field::new("name", DataType::Utf8, false),
        Field::new("embedding_dim", DataType::UInt32, false),
        Field::new("distance_metric", DataType::Utf8, false),
        Field::new("index_method", DataType::Utf8, false),
        Field::new("created_at", DataType::Int64, false),
        Field::new("embedding_source", DataType::Utf8, false),
        Field::new("embedding_version", DataType::UInt32, false),
    ]))
}
fn document_schema() -> SchemaRef {
    Arc::new(Schema::new(vec![
        Field::new("doc_id", DataType::Utf8, false),
        Field::new("external_id", DataType::Utf8, false),
        Field::new("title", DataType::Utf8, false),
        Field::new("mime", DataType::Utf8, false),
        Field::new("source_uri", DataType::Utf8, false),
        Field::new("full_text", DataType::Utf8, false),
        Field::new("keywords", DataType::Utf8, false),
        Field::new("entities", DataType::Utf8, false),
        Field::new("labels", DataType::Utf8, false),
        Field::new("metadata", DataType::Utf8, false),
        Field::new("ingested_at", DataType::Int64, false),
        Field::new("dossier_id", DataType::Utf8, false),
        Field::new("version_ref", DataType::Utf8, false),
    ]))
}
fn chunk_schema(dim: u32) -> SchemaRef {
    Arc::new(Schema::new(vec![
        Field::new("chunk_id", DataType::Utf8, false),
        Field::new("doc_id", DataType::Utf8, false),
        Field::new("external_id", DataType::Utf8, false),
        Field::new("ordinal", DataType::UInt32, false),
        Field::new("content", DataType::Utf8, false),
        Field::new(
            "embedding",
            DataType::FixedSizeList(
                Arc::new(Field::new("item", DataType::Float32, true)),
                dim as i32,
            ),
            false,
        ),
        Field::new("sparse_embedding", DataType::Utf8, false),
        Field::new("multi_vector", DataType::Utf8, false),
        Field::new("chunk_metadata", DataType::Utf8, false),
        Field::new("dossier_id", DataType::Utf8, false),
        Field::new("version_ref", DataType::Utf8, false),
    ]))
}

fn json_string(value: &serde_json::Value) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| "null".to_string())
}
fn metadata_batch(spec: &CollectionSpec) -> RagResult<RecordBatch> {
    RecordBatch::try_new(
        metadata_schema(),
        vec![
            Arc::new(StringArray::from(vec![spec.name.as_str()])),
            Arc::new(UInt32Array::from(vec![spec.embedding_dim])),
            Arc::new(StringArray::from(vec![metric_str(spec.distance_metric)])),
            Arc::new(StringArray::from(vec![index_str(spec.index_method)])),
            Arc::new(Int64Array::from(vec![spec
                .created_at
                .unwrap_or_else(now_secs)])),
            Arc::new(StringArray::from(vec![spec
                .embedding_source
                .as_deref()
                .unwrap_or("")])),
            Arc::new(UInt32Array::from(vec![spec.embedding_version])),
        ],
    )
    .map_err(backend_err)
}
fn document_batch(
    id: &DocumentId,
    doc: &DocumentRecord,
    ingested_at: i64,
) -> RagResult<RecordBatch> {
    RecordBatch::try_new(
        document_schema(),
        vec![
            Arc::new(StringArray::from(vec![id.0.as_str()])),
            Arc::new(StringArray::from(vec![doc
                .external_id
                .as_deref()
                .unwrap_or("")])),
            Arc::new(StringArray::from(vec![doc.title.as_deref().unwrap_or("")])),
            Arc::new(StringArray::from(vec![doc.mime.as_deref().unwrap_or("")])),
            Arc::new(StringArray::from(vec![doc
                .source_uri
                .as_deref()
                .unwrap_or("")])),
            Arc::new(StringArray::from(vec![doc.full_text.as_str()])),
            Arc::new(StringArray::from(vec![json_string(
                &serde_json::to_value(&doc.keywords).unwrap_or_default(),
            )])),
            Arc::new(StringArray::from(vec![json_string(&doc.entities)])),
            Arc::new(StringArray::from(vec![json_string(&doc.labels)])),
            Arc::new(StringArray::from(vec![json_string(&doc.metadata)])),
            Arc::new(Int64Array::from(vec![ingested_at])),
            Arc::new(StringArray::from(vec![doc
                .dossier_id
                .as_deref()
                .unwrap_or("")])),
            Arc::new(StringArray::from(vec![doc
                .version_ref
                .as_deref()
                .unwrap_or("")])),
        ],
    )
    .map_err(backend_err)
}
fn chunk_batch(rows: &[StoredChunk], dim: u32) -> RagResult<RecordBatch> {
    let values = rows
        .iter()
        .map(|r| {
            Some(
                r.record
                    .embedding
                    .iter()
                    .map(|v| Some(*v))
                    .collect::<Vec<_>>(),
            )
        })
        .collect::<Vec<_>>();
    let embedding = FixedSizeListArray::from_iter_primitive::<arrow_array::types::Float32Type, _, _>(
        values, dim as i32,
    );
    RecordBatch::try_new(
        chunk_schema(dim),
        vec![
            Arc::new(StringArray::from(
                rows.iter().map(|r| r.id.0.as_str()).collect::<Vec<_>>(),
            )),
            Arc::new(StringArray::from(
                rows.iter()
                    .map(|r| r.document_id.0.as_str())
                    .collect::<Vec<_>>(),
            )),
            Arc::new(StringArray::from(
                rows.iter()
                    .map(|r| r.record.external_id.as_deref().unwrap_or(""))
                    .collect::<Vec<_>>(),
            )),
            Arc::new(UInt32Array::from(
                rows.iter().map(|r| r.record.ordinal).collect::<Vec<_>>(),
            )),
            Arc::new(StringArray::from(
                rows.iter()
                    .map(|r| r.record.content.as_str())
                    .collect::<Vec<_>>(),
            )),
            Arc::new(embedding),
            Arc::new(StringArray::from(
                rows.iter()
                    .map(|r| {
                        json_string(
                            &serde_json::to_value(&r.record.sparse_embedding).unwrap_or_default(),
                        )
                    })
                    .collect::<Vec<_>>(),
            )),
            Arc::new(StringArray::from(
                rows.iter()
                    .map(|r| {
                        json_string(
                            &serde_json::to_value(&r.record.multi_vector).unwrap_or_default(),
                        )
                    })
                    .collect::<Vec<_>>(),
            )),
            Arc::new(StringArray::from(
                rows.iter()
                    .map(|r| json_string(&r.record.chunk_metadata))
                    .collect::<Vec<_>>(),
            )),
            Arc::new(StringArray::from(
                rows.iter()
                    .map(|r| r.record.dossier_id.as_deref().unwrap_or(""))
                    .collect::<Vec<_>>(),
            )),
            Arc::new(StringArray::from(
                rows.iter()
                    .map(|r| r.record.version_ref.as_deref().unwrap_or(""))
                    .collect::<Vec<_>>(),
            )),
        ],
    )
    .map_err(backend_err)
}

fn column<'a>(batch: &'a RecordBatch, name: &str) -> RagResult<&'a ArrayRef> {
    let i = batch.schema().index_of(name).map_err(backend_err)?;
    Ok(batch.column(i))
}
fn string_at(batch: &RecordBatch, name: &str, row: usize) -> RagResult<String> {
    Ok(column(batch, name)?
        .as_any()
        .downcast_ref::<StringArray>()
        .ok_or_else(|| RagError::InvalidQuery(format!("LanceDB column {name} is not Utf8")))?
        .value(row)
        .to_string())
}
fn uint_at(batch: &RecordBatch, name: &str, row: usize) -> RagResult<u32> {
    Ok(column(batch, name)?
        .as_any()
        .downcast_ref::<UInt32Array>()
        .ok_or_else(|| RagError::InvalidQuery(format!("LanceDB column {name} is not UInt32")))?
        .value(row))
}
fn int_at(batch: &RecordBatch, name: &str, row: usize) -> RagResult<i64> {
    Ok(column(batch, name)?
        .as_any()
        .downcast_ref::<Int64Array>()
        .ok_or_else(|| RagError::InvalidQuery(format!("LanceDB column {name} is not Int64")))?
        .value(row))
}
fn vector_at(batch: &RecordBatch, name: &str, row: usize) -> RagResult<Vec<f32>> {
    let list = column(batch, name)?
        .as_any()
        .downcast_ref::<FixedSizeListArray>()
        .ok_or_else(|| {
            RagError::InvalidQuery(format!("LanceDB column {name} is not a fixed vector"))
        })?;
    let values = list.value(row);
    Ok(values
        .as_any()
        .downcast_ref::<Float32Array>()
        .ok_or_else(|| RagError::InvalidQuery(format!("LanceDB vector {name} is not Float32")))?
        .values()
        .to_vec())
}
fn json_value_at(batch: &RecordBatch, name: &str, row: usize) -> RagResult<serde_json::Value> {
    serde_json::from_str(&string_at(batch, name, row)?).map_err(backend_err)
}
fn json_at<T: serde::de::DeserializeOwned>(
    batch: &RecordBatch,
    name: &str,
    row: usize,
) -> RagResult<T> {
    json_value_at(batch, name, row).and_then(|v| serde_json::from_value(v).map_err(backend_err))
}

fn metric_str(m: DistanceMetric) -> &'static str {
    match m {
        DistanceMetric::Cosine => "cosine",
        DistanceMetric::L2 => "l2",
        DistanceMetric::InnerProduct => "inner_product",
    }
}
fn parse_metric(s: &str) -> RagResult<DistanceMetric> {
    match s {
        "cosine" => Ok(DistanceMetric::Cosine),
        "l2" => Ok(DistanceMetric::L2),
        "inner_product" => Ok(DistanceMetric::InnerProduct),
        _ => Err(RagError::InvalidQuery(format!(
            "unknown LanceDB distance metric: {s}"
        ))),
    }
}
fn index_str(m: IndexMethod) -> &'static str {
    match m {
        IndexMethod::Flat => "flat",
        IndexMethod::Hnsw => "hnsw",
        IndexMethod::Diskann => "diskann",
    }
}
fn parse_index(s: &str) -> RagResult<IndexMethod> {
    match s {
        "flat" => Ok(IndexMethod::Flat),
        "hnsw" => Ok(IndexMethod::Hnsw),
        "diskann" => Ok(IndexMethod::Diskann),
        _ => Err(RagError::InvalidQuery(format!(
            "unknown LanceDB index method: {s}"
        ))),
    }
}
fn dense_score(metric: DistanceMetric, a: &[f32], b: &[f32]) -> f32 {
    match metric {
        DistanceMetric::Cosine => {
            let dot = a.iter().zip(b).map(|(x, y)| x * y).sum::<f32>();
            let na = a.iter().map(|x| x * x).sum::<f32>().sqrt();
            let nb = b.iter().map(|x| x * x).sum::<f32>().sqrt();
            if na == 0.0 || nb == 0.0 {
                0.0
            } else {
                dot / (na * nb)
            }
        }
        DistanceMetric::InnerProduct => a.iter().zip(b).map(|(x, y)| x * y).sum(),
        DistanceMetric::L2 => -a
            .iter()
            .zip(b)
            .map(|(x, y)| (x - y) * (x - y))
            .sum::<f32>()
            .sqrt(),
    }
}
fn text_score(query: &str, content: &str) -> f32 {
    query
        .split_whitespace()
        .filter(|term| {
            let term = term.to_lowercase();
            content.to_lowercase().contains(&term)
        })
        .count() as f32
}
fn summarize(doc: &StoredDocument) -> DocumentSummary {
    DocumentSummary {
        id: doc.id.clone(),
        external_id: doc.record.external_id.clone(),
        title: doc.record.title.clone(),
        mime: doc.record.mime.clone(),
        keywords: doc.record.keywords.clone(),
        labels: doc.record.labels.clone(),
        entities: doc.record.entities.clone(),
        metadata: doc.record.metadata.clone(),
        ingested_at: Some(doc.ingested_at),
        dossier_id: doc.record.dossier_id.clone(),
        version_ref: doc.record.version_ref.clone(),
    }
}

fn resolve_field(
    field: &FilterField,
    doc: &StoredDocument,
    chunk: &ChunkRecord,
) -> Option<serde_json::Value> {
    let parsed = field.parse().ok()?;
    let record = &doc.record;
    match parsed.namespace {
        FilterNamespace::Doc => match parsed.path.as_str() {
            "full_text" => Some(serde_json::Value::String(record.full_text.clone())),
            "title" => record.title.clone().map(serde_json::Value::String),
            "mime" => record.mime.clone().map(serde_json::Value::String),
            "external_id" => record.external_id.clone().map(serde_json::Value::String),
            "source_uri" => record.source_uri.clone().map(serde_json::Value::String),
            "dossier_id" => record.dossier_id.clone().map(serde_json::Value::String),
            "version_ref" => record.version_ref.clone().map(serde_json::Value::String),
            "ingested_at" => Some(serde_json::Value::from(doc.ingested_at)),
            "keywords" => serde_json::to_value(&record.keywords).ok(),
            "labels" => Some(record.labels.clone()),
            "entities" => Some(record.entities.clone()),
            path if path.starts_with("metadata.") => json_pointer(&record.metadata, &path[9..]),
            _ => None,
        },
        FilterNamespace::Chunk => match parsed.path.as_str() {
            "content" => Some(serde_json::Value::String(chunk.content.clone())),
            "ordinal" => Some(serde_json::Value::from(chunk.ordinal)),
            "external_id" => chunk.external_id.clone().map(serde_json::Value::String),
            "dossier_id" => chunk.dossier_id.clone().map(serde_json::Value::String),
            "version_ref" => chunk.version_ref.clone().map(serde_json::Value::String),
            path if path.starts_with("chunk_metadata.") => {
                json_pointer(&chunk.chunk_metadata, &path[15..])
            }
            _ => None,
        },
    }
}
fn json_pointer(value: &serde_json::Value, dotted: &str) -> Option<serde_json::Value> {
    let mut current = value;
    for part in dotted.split('.') {
        current = current.get(part)?;
    }
    Some(current.clone())
}
fn json_cmp(a: &serde_json::Value, b: &serde_json::Value) -> Option<Ordering> {
    match (a.as_f64(), b.as_f64()) {
        (Some(a), Some(b)) => a.partial_cmp(&b),
        _ => Some(a.as_str()?.cmp(b.as_str()?)),
    }
}
fn eval_filter(filter: &Filter, doc: &StoredDocument, chunk: &ChunkRecord) -> bool {
    match filter {
        Filter::Eq { field, value } => resolve_field(field, doc, chunk).as_ref() == Some(value),
        Filter::In { field, values } => resolve_field(field, doc, chunk)
            .is_some_and(|v| values.iter().any(|candidate| candidate == &v)),
        Filter::ArrayContains { field, value } => resolve_field(field, doc, chunk)
            .and_then(|v| v.as_array().cloned())
            .is_some_and(|a| a.iter().any(|v| v == value)),
        Filter::Range {
            field,
            gte,
            gt,
            lte,
            lt,
        } => {
            let Some(v) = resolve_field(field, doc, chunk) else {
                return false;
            };
            let pass = |b: &Option<serde_json::Value>, allowed: &[Ordering]| {
                b.as_ref().is_none_or(|bound| {
                    json_cmp(&v, bound).is_some_and(|ord| allowed.contains(&ord))
                })
            };
            pass(gte, &[Ordering::Greater, Ordering::Equal])
                && pass(gt, &[Ordering::Greater])
                && pass(lte, &[Ordering::Less, Ordering::Equal])
                && pass(lt, &[Ordering::Less])
        }
        Filter::TextMatch { field, query } => resolve_field(field, doc, chunk)
            .and_then(|v| v.as_str().map(|s| s.to_lowercase()))
            .is_some_and(|s| s.contains(&query.to_lowercase())),
        Filter::And { filters } => filters.iter().all(|f| eval_filter(f, doc, chunk)),
        Filter::Or { filters } => filters.iter().any(|f| eval_filter(f, doc, chunk)),
        Filter::Not { filter } => !eval_filter(filter, doc, chunk),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::filter::FilterField;
    use tempfile::tempdir;

    fn chunk(content: &str, ordinal: u32, embedding: Vec<f32>) -> ChunkRecord {
        ChunkRecord {
            external_id: None,
            ordinal,
            content: content.to_string(),
            embedding,
            sparse_embedding: None,
            multi_vector: None,
            chunk_metadata: serde_json::json!({"ordinal": ordinal}),
            dossier_id: None,
            version_ref: None,
        }
    }

    #[test]
    fn schema_mapping_preserves_vector_dimension_and_metadata() {
        let spec = CollectionSpec {
            name: "docs".into(),
            embedding_dim: 3,
            distance_metric: DistanceMetric::Cosine,
            index_method: IndexMethod::Flat,
            created_at: Some(42),
            embedding_source: Some("test-model".into()),
            embedding_version: 7,
        };
        let batch = metadata_batch(&spec).unwrap();
        assert_eq!(batch.schema(), metadata_schema());
        assert_eq!(
            string_at(&batch, "embedding_source", 0).unwrap(),
            "test-model"
        );
        assert_eq!(uint_at(&batch, "embedding_version", 0).unwrap(), 7);
        assert_eq!(
            chunk_schema(3)
                .field_with_name("embedding")
                .unwrap()
                .data_type(),
            &DataType::FixedSizeList(Arc::new(Field::new("item", DataType::Float32, true)), 3)
        );
    }

    #[tokio::test]
    async fn upsert_is_idempotent_and_delete_removes_chunks() {
        let dir = tempdir().unwrap();
        let store = LanceDbStore::connect(dir.path().to_str().unwrap())
            .await
            .unwrap();
        store
            .ensure_collection(&CollectionSpec::new("docs", 2))
            .await
            .unwrap();
        let doc = DocumentRecord {
            external_id: Some("external-1".into()),
            full_text: "alpha".into(),
            ..Default::default()
        };
        let first = store
            .upsert_document("docs", &doc, &[chunk("alpha", 0, vec![1.0, 0.0])])
            .await
            .unwrap();
        let second = store
            .upsert_document("docs", &doc, &[chunk("updated", 0, vec![0.0, 1.0])])
            .await
            .unwrap();
        assert_eq!(first, second);
        assert_eq!(store.collection_stats("docs").await.unwrap().chunks, 1);
        assert_eq!(store.delete_documents("docs", &[first]).await.unwrap(), 1);
        assert_eq!(store.collection_stats("docs").await.unwrap().documents, 0);
        assert_eq!(store.collection_stats("docs").await.unwrap().chunks, 0);
    }

    #[tokio::test]
    async fn replacing_with_no_chunks_removes_only_target_chunks() {
        let dir = tempdir().unwrap();
        let store = LanceDbStore::connect(dir.path().to_str().unwrap())
            .await
            .unwrap();
        store
            .ensure_collection(&CollectionSpec::new("docs", 2))
            .await
            .unwrap();
        let first = DocumentRecord {
            external_id: Some("first".into()),
            ..Default::default()
        };
        let second = DocumentRecord {
            external_id: Some("second".into()),
            ..Default::default()
        };
        let first_id = store
            .upsert_document("docs", &first, &[chunk("old", 0, vec![1.0, 0.0])])
            .await
            .unwrap();
        store
            .upsert_document("docs", &second, &[chunk("keep", 0, vec![0.0, 1.0])])
            .await
            .unwrap();
        assert_eq!(
            store.upsert_document("docs", &first, &[]).await.unwrap(),
            first_id
        );
        assert_eq!(store.collection_stats("docs").await.unwrap().chunks, 1);
    }

    #[tokio::test]
    async fn embedding_update_preserves_chunk_fields_and_ingestion_filter() {
        let dir = tempdir().unwrap();
        let store = LanceDbStore::connect(dir.path().to_str().unwrap())
            .await
            .unwrap();
        store
            .ensure_collection(&CollectionSpec::new("docs", 2))
            .await
            .unwrap();
        let doc = DocumentRecord {
            external_id: Some("one".into()),
            ..Default::default()
        };
        let id = store
            .upsert_document("docs", &doc, &[chunk("unchanged", 0, vec![1.0, 0.0])])
            .await
            .unwrap();
        store
            .update_chunk_embeddings("docs", &id, &[(0, vec![0.0, 1.0])])
            .await
            .unwrap();
        let (_, chunks) = store
            .get_document_chunks("docs", &id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(chunks[0].content, "unchanged");
        assert_eq!(chunks[0].embedding, vec![0.0, 1.0]);
        let output = store
            .retrieve(
                "docs",
                &RetrieveQuery {
                    filter: Some(Filter::Range {
                        field: FilterField("doc.ingested_at".into()),
                        gte: Some(serde_json::json!(0)),
                        gt: None,
                        lte: None,
                        lt: None,
                    }),
                    query_vector: Some(vec![0.0, 1.0]),
                    ..RetrieveQuery::vector(5)
                },
            )
            .await
            .unwrap();
        assert_eq!(output.chunks.len(), 1);
    }

    #[tokio::test]
    async fn retrieval_supports_vector_full_text_hybrid_and_allowlisted_filters() {
        let dir = tempdir().unwrap();
        let store = LanceDbStore::connect(dir.path().to_str().unwrap())
            .await
            .unwrap();
        store
            .ensure_collection(&CollectionSpec::new("docs", 2))
            .await
            .unwrap();
        store
            .upsert_document(
                "docs",
                &DocumentRecord {
                    external_id: Some("one".into()),
                    title: Some("keep".into()),
                    full_text: "alpha beta".into(),
                    ..Default::default()
                },
                &[chunk("alpha", 0, vec![1.0, 0.0])],
            )
            .await
            .unwrap();
        store
            .upsert_document(
                "docs",
                &DocumentRecord {
                    external_id: Some("two".into()),
                    title: Some("drop".into()),
                    full_text: "gamma".into(),
                    ..Default::default()
                },
                &[chunk("gamma", 0, vec![0.0, 1.0])],
            )
            .await
            .unwrap();
        let vector = store
            .retrieve(
                "docs",
                &RetrieveQuery {
                    query_vector: Some(vec![1.0, 0.0]),
                    filter: Some(Filter::Eq {
                        field: FilterField("doc.title".into()),
                        value: serde_json::json!("keep"),
                    }),
                    ..RetrieveQuery::vector(5)
                },
            )
            .await
            .unwrap();
        assert_eq!(vector.chunks.len(), 1);
        let full_text = RetrieveQuery {
            mode: RetrieveMode::FullText,
            query_text: Some("alpha".into()),
            ..RetrieveQuery::vector(5)
        };
        assert_eq!(
            store
                .retrieve("docs", &full_text)
                .await
                .unwrap()
                .chunks
                .len(),
            1
        );
        let hybrid = RetrieveQuery {
            mode: RetrieveMode::Hybrid,
            query_text: Some("alpha".into()),
            query_vector: Some(vec![1.0, 0.0]),
            ..RetrieveQuery::vector(5)
        };
        let hybrid = store.retrieve("docs", &hybrid).await.unwrap();
        assert_eq!(hybrid.chunks[0].content.as_deref(), Some("alpha"));
        let PrimaryScore::Hybrid { rrf, .. } = hybrid.chunks[0].primary_score else {
            panic!("hybrid retrieval reports component scores")
        };
        assert!((rrf - (2.0 / 61.0)).abs() < 1e-6, "{rrf}");
        let PrimaryScore::Hybrid { rrf, .. } = hybrid.chunks[1].primary_score else {
            panic!("hybrid retrieval reports component scores")
        };
        assert!((rrf - (1.0 / 62.0)).abs() < 1e-6, "{rrf}");
        let sparse = RetrieveQuery {
            mode: RetrieveMode::Sparse,
            query_sparse: Some(crate::types::SparseVector {
                indices: vec![1],
                values: vec![1.0],
            }),
            ..RetrieveQuery::vector(5)
        };
        assert!(matches!(
            store.retrieve("docs", &sparse).await,
            Err(RagError::UnsupportedMode { .. })
        ));
    }

    #[tokio::test]
    async fn hybrid_retrieval_requires_a_dense_query_vector() {
        let dir = tempdir().unwrap();
        let store = LanceDbStore::connect(dir.path().to_str().unwrap())
            .await
            .unwrap();
        store
            .ensure_collection(&CollectionSpec::new("docs", 2))
            .await
            .unwrap();

        let query = RetrieveQuery {
            mode: RetrieveMode::Hybrid,
            query_text: Some("alpha".into()),
            ..RetrieveQuery::vector(5)
        };
        assert!(matches!(
            store.retrieve("docs", &query).await,
            Err(RagError::InvalidQuery(message)) if message.contains("requires query_vector")
        ));
    }

    #[tokio::test]
    async fn hybrid_candidate_pool_fuses_only_each_mode_top_candidates() {
        let dir = tempdir().unwrap();
        let store = LanceDbStore::connect(dir.path().to_str().unwrap())
            .await
            .unwrap();
        store
            .ensure_collection(&CollectionSpec::new("docs", 2))
            .await
            .unwrap();
        for (id, content, embedding) in [
            ("dense", "unmatched", vec![1.0, 0.0]),
            ("lexical", "alpha beta", vec![0.5, 0.8660254]),
            ("outside", "alpha", vec![0.0, 1.0]),
        ] {
            store
                .upsert_document(
                    "docs",
                    &DocumentRecord {
                        external_id: Some(id.into()),
                        full_text: content.into(),
                        ..Default::default()
                    },
                    &[chunk(content, 0, embedding)],
                )
                .await
                .unwrap();
        }

        let output = store
            .retrieve(
                "docs",
                &RetrieveQuery {
                    mode: RetrieveMode::Hybrid,
                    query_text: Some("alpha beta".into()),
                    query_vector: Some(vec![1.0, 0.0]),
                    ..RetrieveQuery::vector(1)
                },
            )
            .await
            .unwrap();
        let PrimaryScore::Hybrid { rrf, .. } = output.chunks[0].primary_score else {
            panic!("hybrid retrieval reports component scores")
        };
        assert!((rrf - (1.0 / 61.0)).abs() < 1e-6, "{rrf}");
    }

    #[tokio::test]
    async fn full_text_retrieval_does_not_match_parent_text_outside_chunk() {
        let dir = tempdir().unwrap();
        let store = LanceDbStore::connect(dir.path().to_str().unwrap())
            .await
            .unwrap();
        store
            .ensure_collection(&CollectionSpec::new("docs", 2))
            .await
            .unwrap();
        store
            .upsert_document(
                "docs",
                &DocumentRecord {
                    external_id: Some("doc".into()),
                    full_text: "alpha exists in another chunk".into(),
                    ..Default::default()
                },
                &[chunk("plain paragraph", 0, vec![1.0, 0.0])],
            )
            .await
            .unwrap();

        let output = store
            .retrieve(
                "docs",
                &RetrieveQuery {
                    mode: RetrieveMode::FullText,
                    query_text: Some("alpha".into()),
                    ..RetrieveQuery::vector(5)
                },
            )
            .await
            .unwrap();
        assert!(output.chunks.is_empty());
    }

    #[tokio::test]
    async fn new_empty_collection_has_an_fts_index_and_advertises_text_capability() {
        let dir = tempdir().unwrap();
        let store = LanceDbStore::connect(dir.path().to_str().unwrap())
            .await
            .unwrap();
        store
            .ensure_collection(&CollectionSpec::new("docs", 2))
            .await
            .unwrap();

        let chunks = store
            .open_table(&LanceDbStore::table_name("docs", "chunks"))
            .await
            .unwrap();
        let indices = chunks.list_indices().await.unwrap();
        assert!(indices.iter().any(is_content_fts_index));
        assert!(store.capabilities().full_text);
        let result = store
            .retrieve(
                "docs",
                &RetrieveQuery {
                    mode: RetrieveMode::FullText,
                    query_text: Some("missing".into()),
                    ..RetrieveQuery::vector(5)
                },
            )
            .await
            .unwrap();
        assert!(result.chunks.is_empty());
    }

    #[tokio::test]
    async fn fts_index_survives_reopen_and_upsert_rebuilds_it_once() {
        let dir = tempdir().unwrap();
        let uri = dir.path().to_str().unwrap().to_string();
        let store = LanceDbStore::connect(&uri).await.unwrap();
        store
            .ensure_collection(&CollectionSpec::new("docs", 2))
            .await
            .unwrap();
        let document = DocumentRecord {
            external_id: Some("doc".into()),
            full_text: "alpha".into(),
            ..Default::default()
        };
        store
            .upsert_document("docs", &document, &[chunk("alpha", 0, vec![1.0, 0.0])])
            .await
            .unwrap();
        drop(store);

        let reopened = LanceDbStore::connect(&uri).await.unwrap();
        assert!(reopened.capabilities().full_text);
        let updated = DocumentRecord {
            external_id: Some("doc".into()),
            full_text: "beta".into(),
            ..Default::default()
        };
        reopened
            .upsert_document("docs", &updated, &[chunk("beta", 0, vec![1.0, 0.0])])
            .await
            .unwrap();
        let beta = reopened
            .retrieve(
                "docs",
                &RetrieveQuery {
                    mode: RetrieveMode::FullText,
                    query_text: Some("beta".into()),
                    ..RetrieveQuery::vector(5)
                },
            )
            .await
            .unwrap();
        assert_eq!(beta.chunks.len(), 1);
        assert_eq!(beta.chunks[0].content.as_deref(), Some("beta"));
        let alpha = reopened
            .retrieve(
                "docs",
                &RetrieveQuery {
                    mode: RetrieveMode::FullText,
                    query_text: Some("alpha".into()),
                    ..RetrieveQuery::vector(5)
                },
            )
            .await
            .unwrap();
        assert!(alpha.chunks.is_empty());
        let chunks = reopened
            .open_table(&LanceDbStore::table_name("docs", "chunks"))
            .await
            .unwrap();
        assert_eq!(
            chunks
                .list_indices()
                .await
                .unwrap()
                .into_iter()
                .filter(is_content_fts_index)
                .count(),
            1
        );
    }

    #[tokio::test]
    async fn full_text_uses_bm25_to_rank_chunks_not_documents() {
        let dir = tempdir().unwrap();
        let store = LanceDbStore::connect(dir.path().to_str().unwrap())
            .await
            .unwrap();
        store
            .ensure_collection(&CollectionSpec::new("docs", 2))
            .await
            .unwrap();
        store
            .upsert_document(
                "docs",
                &DocumentRecord {
                    external_id: Some("doc".into()),
                    full_text: "needle needle needle".into(),
                    ..Default::default()
                },
                &[
                    chunk("needle", 0, vec![1.0, 0.0]),
                    chunk("needle needle needle", 1, vec![1.0, 0.0]),
                ],
            )
            .await
            .unwrap();

        let output = store
            .retrieve(
                "docs",
                &RetrieveQuery {
                    mode: RetrieveMode::FullText,
                    query_text: Some("needle".into()),
                    ..RetrieveQuery::vector(2)
                },
            )
            .await
            .unwrap();
        assert_eq!(output.chunks.len(), 2);
        assert_eq!(output.chunks[0].ordinal, 1);
        assert!(output.chunks[0].score > output.chunks[1].score);
        assert!(matches!(
            output.chunks[0].primary_score,
            PrimaryScore::FullText { score } if score > 0.0
        ));
    }

    #[tokio::test]
    async fn full_text_applies_allowlisted_filters_after_bm25_candidates() {
        let dir = tempdir().unwrap();
        let store = LanceDbStore::connect(dir.path().to_str().unwrap())
            .await
            .unwrap();
        store
            .ensure_collection(&CollectionSpec::new("docs", 2))
            .await
            .unwrap();
        for (title, id) in [("keep", "one"), ("drop", "two")] {
            store
                .upsert_document(
                    "docs",
                    &DocumentRecord {
                        external_id: Some(id.into()),
                        title: Some(title.into()),
                        full_text: "needle".into(),
                        ..Default::default()
                    },
                    &[chunk("needle", 0, vec![1.0, 0.0])],
                )
                .await
                .unwrap();
        }

        let output = store
            .retrieve(
                "docs",
                &RetrieveQuery {
                    mode: RetrieveMode::FullText,
                    query_text: Some("needle".into()),
                    filter: Some(Filter::Eq {
                        field: FilterField("doc.title".into()),
                        value: serde_json::json!("keep"),
                    }),
                    include_document: true,
                    ..RetrieveQuery::vector(5)
                },
            )
            .await
            .unwrap();
        assert_eq!(output.chunks.len(), 1);
        assert_eq!(
            output.chunks[0]
                .document
                .as_ref()
                .and_then(|document| document.external_id.as_deref()),
            Some("one")
        );
    }

    /// End-to-end : chunks produits par le chunking côté API (dossier_id /
    /// version_ref à None) héritent du scope de leur document parent à
    /// l'upsert, et une requête dossier/version les retrouve.
    #[tokio::test]
    async fn should_propagate_document_scope_to_chunks_when_chunk_fields_are_none() {
        let dir = tempdir().unwrap();
        let store = LanceDbStore::connect(dir.path().to_str().unwrap())
            .await
            .unwrap();
        store
            .ensure_collection(&CollectionSpec::new("docs", 2))
            .await
            .unwrap();

        let document = DocumentRecord {
            title: Some("contrat".to_string()),
            full_text: "texte du contrat".to_string(),
            dossier_id: Some("dossier-1".to_string()),
            version_ref: Some("v-2024".to_string()),
            ..Default::default()
        };
        // Les chunks n'ont aucun scope : ils doivent l'hériter du document.
        let chunks = vec![chunk("clause de confidentialité", 0, vec![1.0, 0.0])];
        let id = store
            .upsert_document("docs", &document, &chunks)
            .await
            .unwrap();

        // Le document stocké conserve ses scopes.
        let (stored, stored_chunks) = store
            .get_document_chunks("docs", &id)
            .await
            .unwrap()
            .expect("document should exist");
        assert_eq!(stored.dossier_id.as_deref(), Some("dossier-1"));
        assert_eq!(stored.version_ref.as_deref(), Some("v-2024"));
        assert_eq!(stored_chunks[0].dossier_id.as_deref(), Some("dossier-1"));
        assert_eq!(stored_chunks[0].version_ref.as_deref(), Some("v-2024"));

        // Retrieve par dossier : retrouve le chunk malgré des champs chunk
        // vides à l'upsert.
        let output = store
            .retrieve(
                "docs",
                &RetrieveQuery {
                    query_vector: Some(vec![1.0, 0.0]),
                    dossier_id: Some("dossier-1".to_string()),
                    include_content: true,
                    ..RetrieveQuery::vector(10)
                },
            )
            .await
            .unwrap();
        assert_eq!(output.chunks.len(), 1);
        assert_eq!(output.chunks[0].document_id, id);
        assert_eq!(
            output.chunks[0].content.as_deref(),
            Some("clause de confidentialité")
        );

        // Retrieve par version : idem.
        let output = store
            .retrieve(
                "docs",
                &RetrieveQuery {
                    query_vector: Some(vec![1.0, 0.0]),
                    version_ref: Some("v-2024".to_string()),
                    include_content: true,
                    ..RetrieveQuery::vector(10)
                },
            )
            .await
            .unwrap();
        assert_eq!(output.chunks.len(), 1);
        assert_eq!(output.chunks[0].document_id, id);

        // Retrieve par un mauvais dossier : zéro résultat (anti-mélange).
        let output = store
            .retrieve(
                "docs",
                &RetrieveQuery {
                    query_vector: Some(vec![1.0, 0.0]),
                    dossier_id: Some("dossier-9".to_string()),
                    ..RetrieveQuery::vector(10)
                },
            )
            .await
            .unwrap();
        assert!(output.chunks.is_empty());
    }
}
