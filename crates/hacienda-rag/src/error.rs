//! Error model for the RAG base layer.
//!
//! Recovered from `77e2fd3d71^:crates/xberg-rag/src/error.rs` (xberg, MIT).
//! The registry-only variants (`AlreadyRegistered`, `NotRegistered`,
//! `InvalidName`) are dropped: this crate has no process-global registry (see
//! the crate-level doc comment and `superpowers/plans/2026-08-01-hacienda-rag-vector-store-layer.md`
//! Design Decision D3). `Core` is boxed to match hacienda's existing pattern
//! for the same `xberg::XbergError` type (`hacienda-core/src/error.rs`,
//! `hacienda-core/src/pii/mod.rs`), a deliberate deviation from the original
//! crate's unboxed form (Design Decision D4).

use thiserror::Error;

/// Complexity constraint kind for filter validation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ComplexityKind {
    /// Filter nesting depth exceeded.
    Depth,
    /// Total filter node count exceeded.
    NodeCount,
    /// `text_match` predicate count exceeded.
    TextMatchCount,
    /// `text_match` query string byte length exceeded.
    TextMatchQueryBytes,
}

/// Errors raised by vector-store operations.
#[derive(Error, Debug)]
#[non_exhaustive]
pub enum RagError {
    /// The requested collection does not exist.
    #[error("collection not found: {0}")]
    CollectionNotFound(String),

    /// A collection with the same name already exists with a different spec.
    #[error("collection already exists: {0}")]
    CollectionAlreadyExists(String),

    /// Query/chunk embedding dimension does not match the collection.
    #[error("embedding dimension mismatch: expected {expected}, got {got}")]
    EmbeddingDimMismatch {
        /// Dimension declared by the collection.
        expected: u32,
        /// Dimension of the supplied vector.
        got: u32,
    },

    /// An embedder returned a different number of vectors than the inputs it
    /// was given — a broken embedder contract.
    #[error("embedding count mismatch: expected {expected} vectors, got {got}")]
    EmbeddingCountMismatch {
        /// Number of texts submitted for embedding.
        expected: usize,
        /// Number of vectors the embedder returned.
        got: usize,
    },

    /// A filter referenced a field outside the allowed namespaces.
    #[error("filter references unknown field: {field}")]
    FilterUnknownField {
        /// The offending field identifier.
        field: String,
    },

    /// A filter operation does not apply to the field's type.
    #[error("filter type mismatch on field {field}: operation {op} not applicable")]
    FilterTypeMismatch {
        /// The offending field identifier.
        field: String,
        /// The operation that did not apply.
        op: String,
    },

    /// A filter exceeded a complexity limit.
    #[error("filter {kind:?} complexity limit exceeded: cap {cap}, observed {observed}")]
    FilterComplexityExceeded {
        /// Which limit was exceeded.
        kind: ComplexityKind,
        /// The configured cap.
        cap: u32,
        /// The observed value.
        observed: u32,
    },

    /// The query was malformed (bad `top_k`, missing inputs for the mode, …).
    #[error("invalid query: {0}")]
    InvalidQuery(String),

    /// The backend does not support the requested retrieval mode.
    #[error("retrieval mode unsupported by backend '{backend}': {mode}")]
    UnsupportedMode {
        /// The backend's `name()`.
        backend: String,
        /// The requested mode (`full_text`, `hybrid`, …).
        mode: String,
    },

    /// An error originating in xberg core (embeddings, reranking, …).
    ///
    /// Boxed because `xberg::XbergError` is large relative to the other
    /// variants — the same reasoning `HaciendaError`/`PiiError` give for
    /// boxing it (`hacienda-core/src/error.rs`, `hacienda-core/src/pii/mod.rs`).
    #[error(transparent)]
    Core(Box<xberg::XbergError>),

    /// Server-side text chunking (`chunk::chunk_full_text`) failed.
    ///
    /// A dedicated variant rather than routing through [`Self::Core`] via
    /// `#[from]`: a bare `RagError::Core` forwarded straight from `xberg::chunk_text`
    /// gives no indication *which* call produced it once it's crossed into
    /// `hacienda-api`'s `ApiError` and reached a log line — this variant names the
    /// operation and how much text was being split (a length, never the content
    /// itself) alongside the root cause.
    #[error("chunking failed ({input_chars} input chars): {source}")]
    Chunking {
        /// Length of the input text in `char`s — never the text itself.
        input_chars: usize,
        #[source]
        source: Box<xberg::XbergError>,
    },

    /// A backend-specific error from an adapter implementation.
    #[error("backend error: {0}")]
    Backend(#[source] Box<dyn std::error::Error + Send + Sync>),
}

impl From<xberg::XbergError> for RagError {
    fn from(source: xberg::XbergError) -> Self {
        RagError::Core(Box::new(source))
    }
}

/// Result alias for RAG operations.
pub type RagResult<T> = Result<T, RagError>;
