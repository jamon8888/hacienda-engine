//! The backend-agnostic [`RagStore`] contract.
//!
//! Recovered from `77e2fd3d71^:crates/xberg-rag/src/store.rs` (xberg, MIT),
//! renamed from `VectorStore` to `RagStore` (Design Decision D2 in
//! `superpowers/plans/2026-08-01-hacienda-rag-vector-store-layer.md`) to read
//! consistently alongside `AuditStore`/`ReviewStore`/`JobStore` in
//! `hacienda-core` and to avoid a name collision with hacienda's own use of
//! "store" as a suffix convention.
//!
//! This is the extension point the whole RAG layer is built around: this
//! crate ships the trait plus a brute-force in-memory backend, and a durable
//! backend (e.g. tenant-scoped pgvector) is expected to implement it
//! externally. The trait is deliberately **single-tenant**: one instance is
//! one trust domain. Multi-tenancy is layered on top by the caller (e.g. one
//! scoped instance per tenant, or a decorator that sets row-level-security
//! context before delegating) and is never expressed in these signatures —
//! exactly hacienda's own multi-tenancy model.

use crate::capability::Capabilities;
use crate::error::RagResult;
use crate::filter::Filter;
use crate::query::{RetrieveOutput, RetrieveQuery};
use crate::types::{
    ChunkRecord, CollectionSpec, CollectionStats, DocumentId, DocumentRecord, DocumentSummary,
};
use async_trait::async_trait;

/// A vector store: collections of documents and their embedded chunks, queried
/// by vector / full-text / hybrid retrieval.
///
/// # Thread safety
///
/// Implementations are `Send + Sync + 'static` and held behind `Arc<dyn
/// RagStore>`; they may be called concurrently. Backends wrapping
/// non-thread-safe handles must serialize access internally.
///
/// # Obtaining an instance
///
/// Construct an `Arc<dyn RagStore>` directly and inject it — matching how
/// `HaciendaFacade` takes `Arc<dyn AuditStore>`/`Arc<dyn ReviewStore>`/
/// `Arc<dyn JobStore>` by constructor injection. There is no process-global
/// registry (see the crate-level doc comment). Backends that need async,
/// fallible setup must do it in their own constructor and be fully connected
/// before being handed to a caller.
#[async_trait]
pub trait RagStore: Send + Sync + 'static {
    /// A stable identifier for this store instance.
    fn name(&self) -> &str;

    /// What this backend supports (full-text, hybrid, filtering, index methods).
    fn capabilities(&self) -> Capabilities;

    /// Create the collection if it does not exist.
    ///
    /// # Errors
    ///
    /// [`RagError::CollectionAlreadyExists`](crate::RagError::CollectionAlreadyExists)
    /// if a collection with the same name exists with an incompatible spec.
    async fn ensure_collection(&self, spec: &CollectionSpec) -> RagResult<()>;

    /// Drop a collection and all its contents.
    ///
    /// # Errors
    ///
    /// [`RagError::CollectionNotFound`](crate::RagError::CollectionNotFound) if
    /// it does not exist.
    async fn drop_collection(&self, collection: &str) -> RagResult<()>;

    /// Fetch a collection's spec, or `None` if absent.
    ///
    /// # Errors
    ///
    /// Backend errors only.
    async fn get_collection(&self, collection: &str) -> RagResult<Option<CollectionSpec>>;

    /// Upsert a document together with its chunks as one atomic unit.
    ///
    /// Identity is the document's `external_id` when present, otherwise a new
    /// backend-assigned id. Returns the resulting [`DocumentId`].
    ///
    /// # Errors
    ///
    /// [`RagError::EmbeddingDimMismatch`](crate::RagError::EmbeddingDimMismatch)
    /// if any chunk vector does not match the collection dimension.
    async fn upsert_document(
        &self,
        collection: &str,
        document: &DocumentRecord,
        chunks: &[ChunkRecord],
    ) -> RagResult<DocumentId>;

    /// Delete documents (and their chunks) by id. Returns the number removed.
    ///
    /// # Errors
    ///
    /// Backend errors only; unknown ids are ignored.
    async fn delete_documents(&self, collection: &str, ids: &[DocumentId]) -> RagResult<u64>;

    /// Delete documents matching a filter. Returns the number removed.
    ///
    /// # Errors
    ///
    /// Filter or backend errors.
    async fn delete_by_filter(&self, collection: &str, filter: &Filter) -> RagResult<u64>;

    /// Retrieve chunks matching a query.
    ///
    /// # Errors
    ///
    /// [`RagError::UnsupportedMode`](crate::RagError::UnsupportedMode) if the
    /// backend cannot serve the requested [`RetrieveMode`](crate::RetrieveMode);
    /// otherwise validation or backend errors.
    async fn retrieve(&self, collection: &str, query: &RetrieveQuery) -> RagResult<RetrieveOutput>;

    /// Aggregate statistics for a collection.
    ///
    /// # Errors
    ///
    /// [`RagError::CollectionNotFound`](crate::RagError::CollectionNotFound) if
    /// it does not exist.
    async fn collection_stats(&self, collection: &str) -> RagResult<CollectionStats>;

    /// List collections, paginated, newest-first.
    ///
    /// Returns the page of specs together with the total count of collections
    /// *before* `limit`/`offset` are applied, so a caller can tell whether more
    /// pages remain without a second round trip. `limit`/`offset` are honored
    /// by the backend (not the caller) so a store that can push pagination into
    /// its query (e.g. `LIMIT`/`OFFSET` in SQL) never materializes the full set.
    ///
    /// # Errors
    ///
    /// Backend errors only.
    async fn list_collections(
        &self,
        limit: u32,
        offset: u32,
    ) -> RagResult<(Vec<CollectionSpec>, u64)>;

    /// List a collection's documents, paginated, newest-first.
    ///
    /// Same total-before-pagination contract as [`Self::list_collections`].
    ///
    /// # Errors
    ///
    /// [`RagError::CollectionNotFound`](crate::RagError::CollectionNotFound) if
    /// the collection does not exist.
    async fn list_documents(
        &self,
        collection: &str,
        limit: u32,
        offset: u32,
    ) -> RagResult<(Vec<DocumentSummary>, u64)>;

    /// Persist a collection's embedding provenance after a successful
    /// `migrate-embeddings` job.
    ///
    /// A narrow, single-purpose method rather than a general
    /// `update_collection`: the only field of [`CollectionSpec`] any caller
    /// mutates post-creation is embedding provenance (`embedding_source`/
    /// `embedding_version`), and giving that its own verb keeps the trait from
    /// growing an arbitrary partial-update surface whose semantics (which
    /// fields are patchable? what happens to `embedding_dim`?) would need to be
    /// designed and documented for cases that don't exist yet.
    ///
    /// # Errors
    ///
    /// [`RagError::CollectionNotFound`](crate::RagError::CollectionNotFound) if
    /// the collection does not exist.
    async fn set_embedding_provenance(
        &self,
        collection: &str,
        source: &str,
        version: u32,
    ) -> RagResult<()>;

    /// Fetch a document's full record and its chunks (ordinal order), for
    /// pipelines that need to re-derive chunk content/embeddings without going
    /// through `retrieve`'s query path (e.g. `migrate-embeddings`).
    ///
    /// # Errors
    ///
    /// [`RagError::CollectionNotFound`](crate::RagError::CollectionNotFound) if
    /// the collection does not exist. Returns `Ok(None)` if the document itself
    /// does not exist in the collection — not an error, mirrors
    /// [`Self::get_collection`]'s `Option` convention.
    async fn get_document_chunks(
        &self,
        collection: &str,
        document: &DocumentId,
    ) -> RagResult<Option<(DocumentRecord, Vec<ChunkRecord>)>>;

    /// Overwrite the dense embedding vector of specific chunks in place, keyed
    /// by ordinal. Content, metadata, sparse/multi-vector fields, and document
    /// identity are untouched — this is deliberately narrower than
    /// `upsert_document` (see this method's own module context) so a
    /// `migrate-embeddings` job cannot accidentally duplicate a document that
    /// has no `external_id`.
    ///
    /// # Errors
    ///
    /// [`RagError::CollectionNotFound`](crate::RagError::CollectionNotFound) if
    /// the collection does not exist. [`RagError::EmbeddingDimMismatch`](crate::RagError::EmbeddingDimMismatch)
    /// if any vector's length does not match the collection's `embedding_dim`.
    /// Unknown `(document, ordinal)` pairs are silently ignored, mirroring
    /// `delete_documents`'s unknown-id tolerance.
    async fn update_chunk_embeddings(
        &self,
        collection: &str,
        document: &DocumentId,
        embeddings: &[(u32, Vec<f32>)],
    ) -> RagResult<()>;
}
