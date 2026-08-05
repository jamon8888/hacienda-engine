//! # hacienda-rag
//!
//! A backend-agnostic vector-store contract ([`RagStore`]) and a neutral
//! type + filter + query intermediate representation, plus one reference
//! backend ([`InMemoryVectorStore`]).
//!
//! This crate is the engine contract other layers build on: a real deployment
//! implements [`RagStore`] externally (durable pgvector, embedded sqlite, …)
//! while this crate stays single-tenant and free of any product/tenant policy.
//!
//! ## Provenance
//!
//! This crate is recovered, near-verbatim, from xberg's own MIT-licensed
//! `xberg-rag` crate (removed from that workspace pre-1.0 in commit
//! `77e2fd3d711ecf6a673ff1ceb0a17abc9a2c4e64`; recovered from its parent,
//! `77e2fd3d71^`, the crate's final pre-removal state). See the individual
//! module doc comments for the specific adaptations made recovering each
//! file into this workspace (notably: `VectorStore` renamed to [`RagStore`]
//! to read alongside `AuditStore`/`ReviewStore`/`JobStore`, the process-
//! global registry dropped in favor of this codebase's constructor-injection
//! convention, and the `pipeline`/`stream`/`sqlite` modules initially left
//! unrecovered pending a design pass — see
//! `superpowers/plans/2026-08-01-hacienda-rag-vector-store-layer.md`.
//! `stream` was recovered in Phase 12 Track 3 (behind the `streaming`
//! feature); `pipeline`/`sqlite` remain unrecovered.
//! Original xberg license: MIT.
//!
//! ## Scope
//!
//! This crate ships the trait, the neutral types, the filter/query IR, an
//! in-memory reference backend, and (behind the `postgres` feature) a durable
//! `pgvector`-backed `PgVectorStore` (`backends::pgvector`), plus (behind the
//! `streaming` feature) streaming answer synthesis over `liter-llm`
//! (`stream::answer_stream`). Any HTTP surface (`/v1/rag/*` routes) lives
//! outside this crate — see
//! `superpowers/plans/2026-08-01-platform-parity-and-scale-implementation.md`
//! Phase 12 Task 3.

pub mod backends;
pub mod capability;
pub mod error;
pub mod filter;
pub mod query;
mod scoring;
pub mod store;
#[cfg(feature = "streaming")]
pub mod stream;
pub mod types;

pub use backends::memory::InMemoryVectorStore;
pub use capability::Capabilities;
pub use error::{ComplexityKind, RagError, RagResult};
pub use filter::{Filter, FilterField, FilterNamespace};
pub use query::{RetrieveMode, RetrieveOutput, RetrieveQuery};
pub use store::RagStore;
#[cfg(feature = "streaming")]
pub use stream::{
    answer_stream, AnswerEvent, Citation, LlmAnswerConfig, RetrievedContext, TokenUsage,
};
pub use types::{
    ChunkId, ChunkRecord, CollectionSpec, CollectionStats, DistanceMetric, DocumentId,
    DocumentRecord, DocumentSummary, IndexMethod, MultiVector, PrimaryScore, RetrievedChunk,
    SparseVector,
};

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    /// Object-safety check: `RagStore` must be usable behind `Arc<dyn
    /// RagStore>`, the same shape `AuditStore`/`ReviewStore`/`JobStore` already
    /// use in `hacienda-core`. This is a compile-time assertion — it has no
    /// interesting runtime behaviour beyond returning what it was given.
    #[test]
    fn should_stay_object_safe_when_boxed_as_arc_dyn_ragstore() {
        fn assert_object_safe(store: Arc<dyn RagStore>) -> Arc<dyn RagStore> {
            store
        }
        let store: Arc<dyn RagStore> = Arc::new(InMemoryVectorStore::default());
        let _ = assert_object_safe(store);
    }
}
