//! Per-backend capability reporting.
//!
//! Recovered from `77e2fd3d71^:crates/xberg-rag/src/capability.rs` (xberg, MIT).
//! Backends vary in what they support: the in-memory store does vector search
//! only, a durable backend might add full-text + hybrid, and approximate
//! indexes are a further capability on top of that.
//! [`RagStore::capabilities`](crate::RagStore::capabilities) lets callers
//! (and [`RetrieveQuery::validate`](crate::RetrieveQuery::validate)) reject an
//! unsupported mode up front instead of silently degrading.

use crate::types::IndexMethod;
use serde::{Deserialize, Serialize};

/// What a [`RagStore`](crate::RagStore) backend supports.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Capabilities {
    /// Full-text (`RetrieveMode::FullText`) retrieval is supported.
    pub full_text: bool,
    /// Hybrid (`RetrieveMode::Hybrid`) retrieval is supported.
    pub hybrid: bool,
    /// The backend can apply server-side filters during retrieval.
    pub filtering: bool,
    /// Sparse (`RetrieveMode::Sparse`) retrieval is supported.
    pub sparse: bool,
    /// Late-interaction (`RetrieveMode::LateInteraction`) retrieval is supported.
    ///
    /// This flag does not imply identical semantics across backends: a
    /// brute-force in-memory store can do an exhaustive MaxSim scan over
    /// every stored multi-vector (no `query_vector` needed, ignored if
    /// supplied), while a candidate-seeded backend seeds candidates via
    /// dense KNN over `query_vector` (recall bounded by a candidate limit)
    /// and reranks only that set with MaxSim. Same capability, different
    /// recall/latency profile — see
    /// [`RetrieveMode::LateInteraction`](crate::RetrieveMode::LateInteraction).
    pub late_interaction: bool,
    /// Index methods the backend actually implements (others fall back to `Flat`).
    pub index_methods: Vec<IndexMethod>,
}

impl Capabilities {
    /// Vector-only capabilities (the minimal backend: exact vector search, no
    /// full-text, no hybrid, no sparse, no late-interaction). Filtering supported.
    pub fn vector_only() -> Self {
        Self {
            full_text: false,
            hybrid: false,
            filtering: true,
            sparse: false,
            late_interaction: false,
            index_methods: vec![IndexMethod::Flat],
        }
    }
}

impl Default for Capabilities {
    fn default() -> Self {
        Self::vector_only()
    }
}
