//! Pure-Rust, brute-force in-memory vector store.
//!
//! Recovered from `77e2fd3d71^:crates/xberg-rag/src/backends/memory.rs`
//! (xberg, MIT), implementing [`RagStore`] (renamed from `VectorStore`; see
//! `crate::store`). The reference backend: exact vector search over
//! `Vec<f32>`, a compact filter evaluator, and document/chunk bookkeeping. No
//! external dependencies. It does not implement full-text or hybrid
//! retrieval (it has no text index) and reports that via [`Capabilities`].

use crate::capability::Capabilities;
use crate::error::{RagError, RagResult};
use crate::filter::{Filter, FilterField};
use crate::query::{RetrieveMode, RetrieveOutput, RetrieveQuery};
use crate::scoring::{max_sim, sparse_dot};
use crate::store::RagStore;
use crate::types::{
    validate_chunk_side_vectors, ChunkId, ChunkRecord, CollectionSpec, CollectionStats,
    DistanceMetric, DocumentId, DocumentRecord, DocumentSummary, PrimaryScore, RetrievedChunk,
};
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::RwLock;

struct StoredChunk {
    id: ChunkId,
    document_id: DocumentId,
    record: ChunkRecord,
}

struct StoredDocument {
    record: DocumentRecord,
}

#[derive(Default)]
struct Collection {
    spec: Option<CollectionSpec>,
    documents: HashMap<DocumentId, StoredDocument>,
    external_index: HashMap<String, DocumentId>,
    chunks: Vec<StoredChunk>,
}

/// An in-memory [`RagStore`] backed by brute-force scan.
pub struct InMemoryVectorStore {
    name: String,
    collections: RwLock<HashMap<String, Collection>>,
    doc_counter: AtomicU64,
}

impl InMemoryVectorStore {
    /// Create a new, empty in-memory store with the given name.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            collections: RwLock::new(HashMap::new()),
            doc_counter: AtomicU64::new(0),
        }
    }

    fn next_doc_id(&self) -> String {
        let n = self.doc_counter.fetch_add(1, Ordering::Relaxed);
        format!("{}-doc-{n}", self.name)
    }
}

impl Default for InMemoryVectorStore {
    fn default() -> Self {
        Self::new("in-memory")
    }
}

fn score(metric: DistanceMetric, a: &[f32], b: &[f32]) -> f32 {
    match metric {
        DistanceMetric::Cosine => {
            let dot: f32 = a.iter().zip(b).map(|(x, y)| x * y).sum();
            let na: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
            let nb: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();
            if na == 0.0 || nb == 0.0 {
                0.0
            } else {
                dot / (na * nb)
            }
        }
        DistanceMetric::InnerProduct => a.iter().zip(b).map(|(x, y)| x * y).sum(),
        DistanceMetric::L2 => {
            let d2: f32 = a.iter().zip(b).map(|(x, y)| (x - y) * (x - y)).sum();
            -d2.sqrt()
        }
    }
}

/// Resolve a filter field to a JSON value within a (document, chunk) context.
fn resolve_field(
    field: &FilterField,
    doc: &DocumentRecord,
    chunk: &ChunkRecord,
) -> Option<serde_json::Value> {
    let parsed = field.parse().ok()?;
    use crate::filter::FilterNamespace::{Chunk, Doc};
    match parsed.namespace {
        Doc => match parsed.path.as_str() {
            "full_text" => Some(serde_json::Value::String(doc.full_text.clone())),
            "title" => doc.title.clone().map(serde_json::Value::String),
            "mime" => doc.mime.clone().map(serde_json::Value::String),
            "external_id" => doc.external_id.clone().map(serde_json::Value::String),
            "source_uri" => doc.source_uri.clone().map(serde_json::Value::String),
            "keywords" => serde_json::to_value(&doc.keywords).ok(),
            "labels" => Some(doc.labels.clone()),
            "entities" => Some(doc.entities.clone()),
            path if path.starts_with("metadata.") => {
                json_pointer(&doc.metadata, &path["metadata.".len()..])
            }
            _ => None,
        },
        Chunk => match parsed.path.as_str() {
            "content" => Some(serde_json::Value::String(chunk.content.clone())),
            "ordinal" => Some(serde_json::Value::from(chunk.ordinal)),
            "external_id" => chunk.external_id.clone().map(serde_json::Value::String),
            path if path.starts_with("chunk_metadata.") => {
                json_pointer(&chunk.chunk_metadata, &path["chunk_metadata.".len()..])
            }
            _ => None,
        },
    }
}

fn json_pointer(value: &serde_json::Value, dotted: &str) -> Option<serde_json::Value> {
    let mut cur = value;
    for segment in dotted.split('.') {
        cur = cur.get(segment)?;
    }
    Some(cur.clone())
}

fn json_cmp(a: &serde_json::Value, b: &serde_json::Value) -> Option<std::cmp::Ordering> {
    match (a.as_f64(), b.as_f64()) {
        (Some(x), Some(y)) => x.partial_cmp(&y),
        _ => match (a.as_str(), b.as_str()) {
            (Some(x), Some(y)) => Some(x.cmp(y)),
            _ => None,
        },
    }
}

fn eval_filter(filter: &Filter, doc: &DocumentRecord, chunk: &ChunkRecord) -> bool {
    match filter {
        Filter::Eq { field, value } => resolve_field(field, doc, chunk).as_ref() == Some(value),
        Filter::In { field, values } => resolve_field(field, doc, chunk)
            .is_some_and(|v| values.iter().any(|candidate| candidate == &v)),
        Filter::ArrayContains { field, value } => resolve_field(field, doc, chunk)
            .and_then(|v| v.as_array().cloned())
            .is_some_and(|arr| arr.iter().any(|item| item == value)),
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
            use std::cmp::Ordering;
            let pass = |bound: &Option<serde_json::Value>, want: &[Ordering]| {
                bound
                    .as_ref()
                    .is_none_or(|b| json_cmp(&v, b).is_some_and(|ord| want.contains(&ord)))
            };
            pass(gte, &[Ordering::Greater, Ordering::Equal])
                && pass(gt, &[Ordering::Greater])
                && pass(lte, &[Ordering::Less, Ordering::Equal])
                && pass(lt, &[Ordering::Less])
        }
        Filter::TextMatch { field, query } => resolve_field(field, doc, chunk)
            .and_then(|v| v.as_str().map(str::to_string))
            .is_some_and(|s| s.to_lowercase().contains(&query.to_lowercase())),
        Filter::And { filters } => filters.iter().all(|f| eval_filter(f, doc, chunk)),
        Filter::Or { filters } => filters.iter().any(|f| eval_filter(f, doc, chunk)),
        Filter::Not { filter } => !eval_filter(filter, doc, chunk),
    }
}

/// Build a [`RetrievedChunk`] from a stored chunk and a computed score.
fn to_retrieved_chunk(
    coll: &Collection,
    c: &StoredChunk,
    query: &RetrieveQuery,
    score_value: f32,
    primary_score: PrimaryScore,
) -> RetrievedChunk {
    RetrievedChunk {
        id: c.id.clone(),
        document_id: c.document_id.clone(),
        ordinal: c.record.ordinal,
        external_id: c.record.external_id.clone(),
        content: query.include_content.then(|| c.record.content.clone()),
        score: score_value,
        primary_score,
        chunk_metadata: c.record.chunk_metadata.clone(),
        document: query.include_document.then(|| {
            coll.documents
                .get(&c.document_id)
                .map(|d| summarize(&c.document_id, &d.record))
                .unwrap_or_else(|| summarize(&c.document_id, &DocumentRecord::default()))
        }),
    }
}

fn summarize(id: &DocumentId, doc: &DocumentRecord) -> DocumentSummary {
    DocumentSummary {
        id: id.clone(),
        external_id: doc.external_id.clone(),
        title: doc.title.clone(),
        mime: doc.mime.clone(),
        keywords: doc.keywords.clone(),
        labels: doc.labels.clone(),
        entities: doc.entities.clone(),
        metadata: doc.metadata.clone(),
        ingested_at: None,
    }
}

#[async_trait]
impl RagStore for InMemoryVectorStore {
    fn name(&self) -> &str {
        &self.name
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities {
            full_text: false,
            hybrid: false,
            filtering: true,
            sparse: true,
            late_interaction: true,
            index_methods: vec![crate::types::IndexMethod::Flat],
        }
    }

    async fn ensure_collection(&self, spec: &CollectionSpec) -> RagResult<()> {
        let mut collections = self.collections.write().expect("poisoned");
        let entry = collections.entry(spec.name.clone()).or_default();
        match &entry.spec {
            Some(existing) if existing.embedding_dim != spec.embedding_dim => {
                Err(RagError::CollectionAlreadyExists(spec.name.clone()))
            }
            _ => {
                entry.spec = Some(spec.clone());
                Ok(())
            }
        }
    }

    async fn drop_collection(&self, collection: &str) -> RagResult<()> {
        let mut collections = self.collections.write().expect("poisoned");
        collections
            .remove(collection)
            .map(|_| ())
            .ok_or_else(|| RagError::CollectionNotFound(collection.to_string()))
    }

    async fn get_collection(&self, collection: &str) -> RagResult<Option<CollectionSpec>> {
        let collections = self.collections.read().expect("poisoned");
        Ok(collections.get(collection).and_then(|c| c.spec.clone()))
    }

    async fn upsert_document(
        &self,
        collection: &str,
        document: &DocumentRecord,
        chunks: &[ChunkRecord],
    ) -> RagResult<DocumentId> {
        let mut collections = self.collections.write().expect("poisoned");
        let coll = collections
            .get_mut(collection)
            .ok_or_else(|| RagError::CollectionNotFound(collection.to_string()))?;
        let dim = coll.spec.as_ref().map(|s| s.embedding_dim).unwrap_or(0);
        for chunk in chunks {
            if chunk.embedding.len() as u32 != dim {
                return Err(RagError::EmbeddingDimMismatch {
                    expected: dim,
                    got: chunk.embedding.len() as u32,
                });
            }
            validate_chunk_side_vectors(chunk)?;
        }

        let doc_id = match document
            .external_id
            .as_ref()
            .and_then(|ext| coll.external_index.get(ext).cloned())
        {
            Some(existing) => {
                coll.chunks.retain(|c| c.document_id != existing);
                existing
            }
            None => DocumentId(self.next_doc_id()),
        };

        if let Some(ext) = &document.external_id {
            coll.external_index.insert(ext.clone(), doc_id.clone());
        }
        coll.documents.insert(
            doc_id.clone(),
            StoredDocument {
                record: document.clone(),
            },
        );
        for chunk in chunks {
            coll.chunks.push(StoredChunk {
                id: ChunkId(format!("{}:{}", doc_id.0, chunk.ordinal)),
                document_id: doc_id.clone(),
                record: chunk.clone(),
            });
        }
        Ok(doc_id)
    }

    async fn delete_documents(&self, collection: &str, ids: &[DocumentId]) -> RagResult<u64> {
        let mut collections = self.collections.write().expect("poisoned");
        let coll = collections
            .get_mut(collection)
            .ok_or_else(|| RagError::CollectionNotFound(collection.to_string()))?;
        let mut removed = 0u64;
        for id in ids {
            if let Some(doc) = coll.documents.remove(id) {
                removed += 1;
                if let Some(ext) = doc.record.external_id {
                    coll.external_index.remove(&ext);
                }
            }
        }
        coll.chunks.retain(|c| !ids.contains(&c.document_id));
        Ok(removed)
    }

    async fn delete_by_filter(&self, collection: &str, filter: &Filter) -> RagResult<u64> {
        filter.validate()?;
        let mut collections = self.collections.write().expect("poisoned");
        let coll = collections
            .get_mut(collection)
            .ok_or_else(|| RagError::CollectionNotFound(collection.to_string()))?;
        let mut to_remove: Vec<DocumentId> = Vec::new();
        for (id, doc) in &coll.documents {
            let matches = coll
                .chunks
                .iter()
                .filter(|c| &c.document_id == id)
                .any(|c| eval_filter(filter, &doc.record, &c.record));
            if matches {
                to_remove.push(id.clone());
            }
        }
        let mut removed = 0u64;
        for id in &to_remove {
            if let Some(doc) = coll.documents.remove(id) {
                removed += 1;
                if let Some(ext) = doc.record.external_id {
                    coll.external_index.remove(&ext);
                }
            }
        }
        coll.chunks.retain(|c| !to_remove.contains(&c.document_id));
        Ok(removed)
    }

    async fn retrieve(&self, collection: &str, query: &RetrieveQuery) -> RagResult<RetrieveOutput> {
        if !matches!(
            query.mode,
            RetrieveMode::Vector | RetrieveMode::Sparse | RetrieveMode::LateInteraction
        ) {
            return Err(RagError::UnsupportedMode {
                backend: self.name.clone(),
                mode: query.mode.as_str().to_string(),
            });
        }
        let collections = self.collections.read().expect("poisoned");
        let coll = collections
            .get(collection)
            .ok_or_else(|| RagError::CollectionNotFound(collection.to_string()))?;
        let spec = coll
            .spec
            .as_ref()
            .ok_or_else(|| RagError::CollectionNotFound(collection.to_string()))?;
        query.validate(spec)?;

        let candidates: Vec<&StoredChunk> = coll
            .chunks
            .iter()
            .filter(|c| {
                query.filter.as_ref().is_none_or(|f| {
                    coll.documents
                        .get(&c.document_id)
                        .is_some_and(|d| eval_filter(f, &d.record, &c.record))
                })
            })
            .collect();

        let mut scored: Vec<RetrievedChunk> = match query.mode {
            RetrieveMode::Vector => {
                let query_vector = query.query_vector.as_ref().ok_or_else(|| {
                    RagError::InvalidQuery(
                        "in-memory backend cannot embed text; supply query_vector".to_string(),
                    )
                })?;
                candidates
                    .iter()
                    .map(|c| {
                        let s = score(spec.distance_metric, query_vector, &c.record.embedding);
                        to_retrieved_chunk(coll, c, query, s, PrimaryScore::Vector { score: s })
                    })
                    .collect()
            }
            RetrieveMode::Sparse => {
                let query_sparse = query
                    .query_sparse
                    .as_ref()
                    .expect("validated: query_sparse present");
                candidates
                    .iter()
                    .filter_map(|c| {
                        let doc_sparse = c.record.sparse_embedding.as_ref()?;
                        let s = sparse_dot(query_sparse, doc_sparse);
                        (s > 0.0).then(|| {
                            to_retrieved_chunk(coll, c, query, s, PrimaryScore::Sparse { score: s })
                        })
                    })
                    .collect()
            }
            RetrieveMode::LateInteraction => {
                let query_mv = query
                    .query_multi_vector
                    .as_ref()
                    .expect("validated: query_multi_vector present");
                candidates
                    .iter()
                    .filter_map(|c| {
                        let doc_mv = c.record.multi_vector.as_ref()?;
                        let s = max_sim(query_mv, doc_mv);
                        Some(to_retrieved_chunk(
                            coll,
                            c,
                            query,
                            s,
                            PrimaryScore::LateInteraction { score: s },
                        ))
                    })
                    .collect()
            }
            RetrieveMode::FullText | RetrieveMode::Hybrid => {
                unreachable!("guarded by capability check above")
            }
        };

        scored.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        if query.group_by_document {
            let mut seen: HashMap<DocumentId, ()> = HashMap::new();
            scored.retain(|c| seen.insert(c.document_id.clone(), ()).is_none());
        }
        scored.truncate(query.top_k as usize);

        Ok(RetrieveOutput {
            mode: query.mode,
            chunks: scored,
            primary_latency_ms: 0,
        })
    }

    async fn collection_stats(&self, collection: &str) -> RagResult<CollectionStats> {
        let collections = self.collections.read().expect("poisoned");
        let coll = collections
            .get(collection)
            .ok_or_else(|| RagError::CollectionNotFound(collection.to_string()))?;
        Ok(CollectionStats {
            documents: coll.documents.len() as u64,
            chunks: coll.chunks.len() as u64,
            last_ingested_at: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn store_with_collection(dim: u32) -> InMemoryVectorStore {
        let store = InMemoryVectorStore::new("test");
        store
            .ensure_collection(&CollectionSpec::new("docs", dim))
            .await
            .unwrap();
        store
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

    fn chunk_full(
        ordinal: u32,
        content: &str,
        embedding: Vec<f32>,
        sparse_embedding: Option<crate::types::SparseVector>,
        multi_vector: Option<crate::types::MultiVector>,
    ) -> ChunkRecord {
        ChunkRecord {
            external_id: None,
            ordinal,
            content: content.to_string(),
            embedding,
            sparse_embedding,
            multi_vector,
            chunk_metadata: serde_json::Value::Null,
        }
    }

    #[tokio::test]
    async fn should_order_by_similarity_when_retrieving_after_upsert() {
        let store = store_with_collection(3).await;
        let doc = DocumentRecord {
            full_text: "hello world".to_string(),
            ..Default::default()
        };
        let chunks = vec![
            chunk(0, "near", vec![1.0, 0.0, 0.0]),
            chunk(1, "far", vec![0.0, 1.0, 0.0]),
        ];
        store.upsert_document("docs", &doc, &chunks).await.unwrap();

        let q = RetrieveQuery {
            query_vector: Some(vec![1.0, 0.0, 0.0]),
            ..RetrieveQuery::vector(10)
        };
        let out = store.retrieve("docs", &q).await.unwrap();
        assert_eq!(out.chunks.len(), 2);
        assert_eq!(out.chunks[0].content.as_deref(), Some("near"));
    }

    #[tokio::test]
    async fn should_reject_upsert_when_embedding_dimension_mismatches() {
        let store = store_with_collection(3).await;
        let doc = DocumentRecord::default();
        let bad = vec![chunk(0, "x", vec![1.0, 0.0])];
        let err = store.upsert_document("docs", &doc, &bad).await.unwrap_err();
        assert!(matches!(
            err,
            RagError::EmbeddingDimMismatch {
                expected: 3,
                got: 2
            }
        ));
    }

    #[tokio::test]
    async fn should_reject_retrieve_when_full_text_mode_is_unsupported() {
        let store = store_with_collection(3).await;
        let mut q = RetrieveQuery::vector(5);
        q.mode = RetrieveMode::FullText;
        q.query_text = Some("hi".to_string());
        let err = store.retrieve("docs", &q).await.unwrap_err();
        assert!(matches!(err, RagError::UnsupportedMode { .. }));
    }

    #[tokio::test]
    async fn should_reject_retrieve_when_hybrid_mode_is_unsupported() {
        let store = store_with_collection(3).await;
        let mut q = RetrieveQuery::vector(5);
        q.mode = RetrieveMode::Hybrid;
        q.query_text = Some("hi".to_string());
        let err = store.retrieve("docs", &q).await.unwrap_err();
        assert!(matches!(
            err,
            RagError::UnsupportedMode { ref mode, .. } if mode == "hybrid"
        ));
    }

    #[tokio::test]
    async fn should_constrain_results_when_filter_is_applied() {
        let store = store_with_collection(2).await;
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
            .upsert_document("docs", &a, &[chunk(0, "a", vec![1.0, 0.0])])
            .await
            .unwrap();
        store
            .upsert_document("docs", &b, &[chunk(0, "b", vec![1.0, 0.0])])
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
        let out = store.retrieve("docs", &q).await.unwrap();
        assert_eq!(out.chunks.len(), 1);
    }

    #[tokio::test]
    async fn should_rank_by_overlap_when_retrieving_sparse() {
        use crate::types::SparseVector;
        let store = store_with_collection(2).await;
        let doc = DocumentRecord::default();
        let chunks = vec![
            chunk_full(
                0,
                "match",
                vec![1.0, 0.0],
                Some(SparseVector {
                    indices: vec![1, 3],
                    values: vec![2.0, 2.0],
                }),
                None,
            ),
            chunk_full(
                1,
                "miss",
                vec![0.0, 1.0],
                Some(SparseVector {
                    indices: vec![5, 9],
                    values: vec![2.0, 2.0],
                }),
                None,
            ),
        ];
        store.upsert_document("docs", &doc, &chunks).await.unwrap();

        let q = RetrieveQuery {
            mode: RetrieveMode::Sparse,
            query_sparse: Some(SparseVector {
                indices: vec![1, 3],
                values: vec![1.0, 1.0],
            }),
            include_content: true,
            ..RetrieveQuery::vector(10)
        };
        let out = store.retrieve("docs", &q).await.unwrap();
        assert_eq!(out.chunks.len(), 1, "only the overlapping chunk scores > 0");
        assert_eq!(out.chunks[0].content.as_deref(), Some("match"));
    }

    #[tokio::test]
    async fn should_rank_by_max_sim_when_retrieving_late_interaction() {
        use crate::types::MultiVector;
        let store = store_with_collection(2).await;
        let doc = DocumentRecord::default();
        let chunks = vec![
            chunk_full(
                0,
                "aligned",
                vec![1.0, 0.0],
                None,
                Some(MultiVector {
                    num_tokens: 1,
                    dim: 2,
                    data: vec![1.0, 0.0],
                }),
            ),
            chunk_full(
                1,
                "orthogonal",
                vec![0.0, 1.0],
                None,
                Some(MultiVector {
                    num_tokens: 1,
                    dim: 2,
                    data: vec![0.0, 1.0],
                }),
            ),
        ];
        store.upsert_document("docs", &doc, &chunks).await.unwrap();

        let q = RetrieveQuery {
            mode: RetrieveMode::LateInteraction,
            query_vector: Some(vec![1.0, 0.0]),
            query_multi_vector: Some(MultiVector {
                num_tokens: 1,
                dim: 2,
                data: vec![1.0, 0.0],
            }),
            include_content: true,
            ..RetrieveQuery::vector(10)
        };
        let out = store.retrieve("docs", &q).await.unwrap();
        assert_eq!(out.chunks[0].content.as_deref(), Some("aligned"));
    }

    #[tokio::test]
    async fn should_replace_chunks_when_external_id_upsert_repeats() {
        let store = store_with_collection(2).await;
        let doc = DocumentRecord {
            external_id: Some("ext-1".to_string()),
            ..Default::default()
        };
        store
            .upsert_document("docs", &doc, &[chunk(0, "v1", vec![1.0, 0.0])])
            .await
            .unwrap();
        store
            .upsert_document("docs", &doc, &[chunk(0, "v2", vec![0.0, 1.0])])
            .await
            .unwrap();
        let stats = store.collection_stats("docs").await.unwrap();
        assert_eq!(stats.documents, 1);
        assert_eq!(stats.chunks, 1);
    }

    #[tokio::test]
    async fn should_be_idempotent_when_ensure_collection_repeats_matching_dim() {
        let store = store_with_collection(4).await;
        store
            .ensure_collection(&CollectionSpec::new("docs", 4))
            .await
            .unwrap();
        let spec = store.get_collection("docs").await.unwrap();
        assert_eq!(spec.map(|s| s.embedding_dim), Some(4));
    }

    #[tokio::test]
    async fn should_reject_ensure_collection_when_dim_mismatches_existing() {
        let store = store_with_collection(4).await;
        let err = store
            .ensure_collection(&CollectionSpec::new("docs", 8))
            .await
            .unwrap_err();
        assert!(matches!(err, RagError::CollectionAlreadyExists(ref name) if name == "docs"));
    }

    #[tokio::test]
    async fn should_remove_collection_when_dropped() {
        let store = store_with_collection(2).await;
        store.drop_collection("docs").await.unwrap();
        assert!(store.get_collection("docs").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn should_reject_drop_when_collection_is_unknown() {
        let store = InMemoryVectorStore::new("test");
        let err = store.drop_collection("missing").await.unwrap_err();
        assert!(matches!(err, RagError::CollectionNotFound(ref name) if name == "missing"));
    }

    #[tokio::test]
    async fn should_return_spec_when_get_collection_exists() {
        let store = store_with_collection(4).await;
        let spec = store.get_collection("docs").await.unwrap();
        assert_eq!(spec.map(|s| s.embedding_dim), Some(4));
    }

    #[tokio::test]
    async fn should_return_none_when_get_collection_is_unknown() {
        let store = InMemoryVectorStore::new("test");
        assert!(store.get_collection("missing").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn should_remove_by_id_when_deleting_documents() {
        let store = store_with_collection(2).await;
        let doc = DocumentRecord::default();
        let id = store
            .upsert_document("docs", &doc, &[chunk(0, "a", vec![1.0, 0.0])])
            .await
            .unwrap();

        let removed = store
            .delete_documents("docs", std::slice::from_ref(&id))
            .await
            .unwrap();
        assert_eq!(removed, 1);
        let stats = store.collection_stats("docs").await.unwrap();
        assert_eq!(stats.documents, 0);
        assert_eq!(stats.chunks, 0);
    }

    #[tokio::test]
    async fn should_ignore_unknown_ids_when_deleting_documents() {
        let store = store_with_collection(2).await;
        let removed = store
            .delete_documents("docs", &[DocumentId("does-not-exist".to_string())])
            .await
            .unwrap();
        assert_eq!(removed, 0);
    }

    #[tokio::test]
    async fn should_remove_matching_documents_when_deleting_by_filter() {
        let store = store_with_collection(2).await;
        let keep = DocumentRecord {
            title: Some("keep".to_string()),
            ..Default::default()
        };
        let drop = DocumentRecord {
            title: Some("drop".to_string()),
            ..Default::default()
        };
        store
            .upsert_document("docs", &keep, &[chunk(0, "a", vec![1.0, 0.0])])
            .await
            .unwrap();
        store
            .upsert_document("docs", &drop, &[chunk(0, "b", vec![0.0, 1.0])])
            .await
            .unwrap();

        let removed = store
            .delete_by_filter(
                "docs",
                &Filter::Eq {
                    field: FilterField("doc.title".to_string()),
                    value: serde_json::json!("drop"),
                },
            )
            .await
            .unwrap();
        assert_eq!(removed, 1);
        let stats = store.collection_stats("docs").await.unwrap();
        assert_eq!(stats.documents, 1);
    }

    #[tokio::test]
    async fn should_report_aggregate_stats_when_collection_has_documents() {
        let store = store_with_collection(2).await;
        store
            .upsert_document(
                "docs",
                &DocumentRecord::default(),
                &[chunk(0, "a", vec![1.0, 0.0])],
            )
            .await
            .unwrap();
        store
            .upsert_document(
                "docs",
                &DocumentRecord::default(),
                &[chunk(0, "b", vec![0.0, 1.0]), chunk(1, "c", vec![1.0, 1.0])],
            )
            .await
            .unwrap();

        let stats = store.collection_stats("docs").await.unwrap();
        assert_eq!(stats.documents, 2);
        assert_eq!(stats.chunks, 3);
    }

    #[tokio::test]
    async fn should_reject_collection_stats_when_collection_is_unknown() {
        let store = InMemoryVectorStore::new("test");
        let err = store.collection_stats("missing").await.unwrap_err();
        assert!(matches!(err, RagError::CollectionNotFound(ref name) if name == "missing"));
    }
}
