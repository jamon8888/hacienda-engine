//! Token-only text embedding boundary.
//!
//! Retrieval must never send source text containing plaintext PII to an
//! embedding backend.  [`TokenOnlyText`] makes that boundary visible in the
//! type carried by this module.  The type deliberately does not implement
//! `Serialize`, `Display`, or a content-bearing `Debug` implementation, so a
//! token-only value cannot accidentally become a log or wire payload through
//! a generic formatting path.
//!
//! The [`XbergTokenEmbedder`] adapter (behind this crate's `embeddings`
//! feature) delegates to Xberg's real embedding implementation.  There is no
//! deterministic test/fallback embedder in production code.

use crate::types::CollectionSpec;
use async_trait::async_trait;
use std::fmt;
use thiserror::Error;

/// Text after the caller's redaction/tokenisation step.
///
/// Construct this value only from text that is already token-only.  The
/// constructor's name is intentionally explicit: this module cannot prove
/// that an arbitrary string has been redacted, but it can ensure that every
/// embedding API accepts this marker type rather than an unconstrained
/// `String` or `&str`.
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct TokenOnlyText(String);

impl TokenOnlyText {
    /// Mark already-tokenised text as safe for embedding.
    ///
    /// This does not inspect or transform the value.  Callers must pass the
    /// token-only projection produced by the secrecy/redaction pipeline.
    pub fn from_token_only(text: impl Into<String>) -> Self {
        Self(text.into())
    }

    /// Borrow the token-only text for an embedding backend call.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl AsRef<str> for TokenOnlyText {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl fmt::Debug for TokenOnlyText {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("TokenOnlyText(<redacted>)")
    }
}

/// Stable identity and shape information for an embedding model.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmbeddingMetadata {
    provenance: EmbeddingProvenance,
    dimension: u32,
    batch_size: usize,
}

impl EmbeddingMetadata {
    /// Describe an embedder's provenance, vector dimension, and batch limit.
    pub fn new(
        source: impl Into<String>,
        version: u32,
        dimension: u32,
        batch_size: usize,
    ) -> Result<Self, EmbeddingError> {
        if dimension == 0 {
            return Err(EmbeddingError::InvalidDimension { dimension });
        }
        if batch_size == 0 {
            return Err(EmbeddingError::InvalidBatchSize { batch_size });
        }
        Ok(Self {
            provenance: EmbeddingProvenance {
                source: source.into(),
                version,
            },
            dimension,
            batch_size,
        })
    }

    /// Source/model identifier to persist in [`CollectionSpec`].
    pub fn source(&self) -> &str {
        &self.provenance.source
    }

    /// Version of the source/model contract.
    pub const fn version(&self) -> u32 {
        self.provenance.version
    }

    /// Number of scalar values in each dense vector.
    pub const fn dimension(&self) -> u32 {
        self.dimension
    }

    /// Maximum number of token-only texts sent to one backend call.
    pub const fn batch_size(&self) -> usize {
        self.batch_size
    }

    /// Build collection metadata that is compatible with this embedder.
    pub fn collection_spec(&self, name: impl Into<String>) -> CollectionSpec {
        CollectionSpec {
            name: name.into(),
            embedding_dim: self.dimension,
            distance_metric: Default::default(),
            index_method: Default::default(),
            created_at: None,
            embedding_source: Some(self.source().to_owned()),
            embedding_version: self.version(),
        }
    }
}

/// The model/source identity and version persisted alongside a collection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmbeddingProvenance {
    /// Stable model/backend identifier, never source document content.
    pub source: String,
    /// Monotonic source version used by migration jobs.
    pub version: u32,
}

/// Errors at the text-embedding boundary.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum EmbeddingError {
    /// No texts were supplied.  Empty requests are rejected before touching a
    /// backend so they cannot be mistaken for a successful retrieval query.
    #[error("cannot embed an empty input batch")]
    EmptyInput,

    /// A token-only text value was empty.
    #[error("token-only text at position {position} is empty")]
    EmptyText { position: usize },

    /// The configured batch size must be positive.
    #[error("embedding batch size must be greater than zero (got {batch_size})")]
    InvalidBatchSize { batch_size: usize },

    /// The configured vector dimension must be positive.
    #[error("embedding dimension must be greater than zero (got {dimension})")]
    InvalidDimension { dimension: u32 },

    /// A backend returned a different number of vectors than inputs.
    #[error("embedding count mismatch: expected {expected}, got {got}")]
    CountMismatch { expected: usize, got: usize },

    /// A backend call exceeded the adapter's declared batch size.
    #[error("embedding batch too large: maximum {maximum}, got {actual}")]
    BatchTooLarge { maximum: usize, actual: usize },

    /// A backend returned a vector with the wrong shape.
    #[error("embedding dimension mismatch at vector {index}: expected {expected}, got {got}")]
    DimensionMismatch {
        /// Zero-based vector position in the submitted batch.
        index: usize,
        /// Declared model dimension.
        expected: u32,
        /// Returned vector dimension.
        got: u32,
    },

    /// A backend/model could not provide a collection-safe dimension.
    #[error("embedding model has no known vector dimension")]
    DimensionUnavailable,

    /// A real embedding backend failed.  The source is retained for callers,
    /// while this variant's display text contains no input text.
    #[error("embedding backend failed")]
    Backend(#[source] Box<dyn std::error::Error + Send + Sync>),
}

/// Async boundary for dense text embeddings.
///
/// Implementations expose only [`TokenOnlyText`] values.  `embed` performs
/// bounded batching and validates count/dimension for every backend response;
/// implementations only need to implement one backend-sized call.
#[async_trait]
pub trait TokenTextEmbedder: Send + Sync {
    /// Model identity and shape used to create a compatible collection.
    fn metadata(&self) -> &EmbeddingMetadata;

    /// Embed one backend-sized batch.  Implementations must not accept a batch
    /// larger than `metadata().batch_size()`.
    async fn embed_batch(&self, inputs: &[TokenOnlyText]) -> Result<Vec<Vec<f32>>, EmbeddingError>;

    /// Embed token-only texts in bounded batches, preserving input order.
    async fn embed(&self, inputs: &[TokenOnlyText]) -> Result<Vec<Vec<f32>>, EmbeddingError> {
        if inputs.is_empty() {
            return Err(EmbeddingError::EmptyInput);
        }

        for (position, input) in inputs.iter().enumerate() {
            if input.as_str().is_empty() {
                return Err(EmbeddingError::EmptyText {
                    position: position + 1,
                });
            }
        }

        let batch_size = self.metadata().batch_size();
        if batch_size == 0 {
            return Err(EmbeddingError::InvalidBatchSize { batch_size });
        }

        let mut output = Vec::with_capacity(inputs.len());
        for batch in inputs.chunks(batch_size) {
            let vectors = self.embed_batch(batch).await?;
            validate_batch(batch.len(), self.metadata().dimension(), vectors.as_slice())?;
            output.extend(vectors);
        }
        Ok(output)
    }
}

/// Compatibility name for callers that prefer the shorter interface name.
pub use TokenTextEmbedder as TextEmbedder;

/// Validate one backend response without logging any input text.
pub fn validate_batch(
    expected_count: usize,
    expected_dimension: u32,
    vectors: &[Vec<f32>],
) -> Result<(), EmbeddingError> {
    if vectors.len() != expected_count {
        return Err(EmbeddingError::CountMismatch {
            expected: expected_count,
            got: vectors.len(),
        });
    }
    for (index, vector) in vectors.iter().enumerate() {
        let got = u32::try_from(vector.len()).unwrap_or(u32::MAX);
        if got != expected_dimension {
            return Err(EmbeddingError::DimensionMismatch {
                index,
                expected: expected_dimension,
                got,
            });
        }
    }
    Ok(())
}

/// Adapter around Xberg's real dense embedding implementation.
#[cfg(feature = "embeddings")]
pub struct XbergTokenEmbedder {
    config: xberg::EmbeddingConfig,
    metadata: EmbeddingMetadata,
}

#[cfg(feature = "embeddings")]
impl XbergTokenEmbedder {
    /// Construct an adapter when the caller has an explicit model dimension.
    ///
    /// The explicit dimension is useful for Xberg plugin/hosted models whose
    /// provider owns the model metadata.  Preset and custom models can use
    /// [`Self::from_config`] to resolve it from Xberg's public metadata.
    pub fn new(
        config: xberg::EmbeddingConfig,
        source: impl Into<String>,
        version: u32,
        dimension: u32,
    ) -> Result<Self, EmbeddingError> {
        let batch_size = if config.batch_size == 0 {
            32
        } else {
            config.batch_size
        };
        let metadata = EmbeddingMetadata::new(source, version, dimension, batch_size)?;
        Ok(Self { config, metadata })
    }

    /// Construct an adapter from a preset or custom Xberg model config.
    pub fn from_config(
        config: xberg::EmbeddingConfig,
        version: u32,
    ) -> Result<Self, EmbeddingError> {
        let (source, dimension) = match &config.model {
            xberg::EmbeddingModelType::Preset { name } => {
                let preset = xberg::embeddings::EMBEDDING_PRESETS
                    .iter()
                    .find(|preset| preset.name == *name)
                    .ok_or(EmbeddingError::DimensionUnavailable)?;
                (format!("xberg:preset:{name}"), preset.dimensions)
            }
            xberg::EmbeddingModelType::Custom {
                model_id,
                dimensions,
            } => (format!("xberg:custom:{model_id}"), *dimensions),
            xberg::EmbeddingModelType::Llm { .. } | xberg::EmbeddingModelType::Plugin { .. } => {
                return Err(EmbeddingError::DimensionUnavailable);
            }
        };
        let dimension = u32::try_from(dimension).map_err(|_| EmbeddingError::InvalidDimension {
            dimension: u32::MAX,
        })?;
        Self::new(config, source, version, dimension)
    }
}

#[cfg(feature = "embeddings")]
#[async_trait]
impl TokenTextEmbedder for XbergTokenEmbedder {
    fn metadata(&self) -> &EmbeddingMetadata {
        &self.metadata
    }

    async fn embed_batch(&self, inputs: &[TokenOnlyText]) -> Result<Vec<Vec<f32>>, EmbeddingError> {
        if inputs.is_empty() {
            return Err(EmbeddingError::EmptyInput);
        }
        if inputs.len() > self.metadata.batch_size() {
            return Err(EmbeddingError::BatchTooLarge {
                maximum: self.metadata.batch_size(),
                actual: inputs.len(),
            });
        }
        let texts = inputs
            .iter()
            .map(|input| input.as_str().to_owned())
            .collect::<Vec<_>>();
        xberg::embed_texts_async(texts, &self.config)
            .await
            .map_err(|error| EmbeddingError::Backend(Box::new(error)))
    }
}

#[cfg(feature = "embeddings")]
pub use XbergTokenEmbedder as XbergTextEmbedder;

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    struct TestEmbedder {
        metadata: EmbeddingMetadata,
        dimension: usize,
        wrong_count: bool,
        calls: Arc<Mutex<Vec<usize>>>,
    }

    #[async_trait]
    impl TokenTextEmbedder for TestEmbedder {
        fn metadata(&self) -> &EmbeddingMetadata {
            &self.metadata
        }

        async fn embed_batch(
            &self,
            inputs: &[TokenOnlyText],
        ) -> Result<Vec<Vec<f32>>, EmbeddingError> {
            self.calls.lock().expect("test mutex").push(inputs.len());
            let count = if self.wrong_count {
                inputs.len().saturating_sub(1)
            } else {
                inputs.len()
            };
            Ok((0..count).map(|_| vec![0.0; self.dimension]).collect())
        }
    }

    fn input(count: usize) -> Vec<TokenOnlyText> {
        (0..count)
            .map(|index| TokenOnlyText::from_token_only(format!("[TOKEN:{index}]")))
            .collect()
    }

    fn test_embedder(batch_size: usize, dimension: usize, wrong_count: bool) -> TestEmbedder {
        TestEmbedder {
            metadata: EmbeddingMetadata::new("test:embedder", 1, dimension as u32, batch_size)
                .expect("valid test metadata"),
            dimension,
            wrong_count,
            calls: Arc::new(Mutex::new(Vec::new())),
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn rejects_empty_input_before_backend_call() {
        let embedder = test_embedder(2, 3, false);
        let error = embedder
            .embed(&[])
            .await
            .expect_err("empty input must fail");
        assert!(matches!(error, EmbeddingError::EmptyInput));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn batches_and_preserves_vector_count() {
        let embedder = test_embedder(2, 3, false);
        let calls = Arc::clone(&embedder.calls);
        let vectors = embedder.embed(&input(5)).await.expect("embedding succeeds");
        assert_eq!(vectors.len(), 5);
        assert_eq!(*calls.lock().expect("test mutex"), vec![2, 2, 1]);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn rejects_backend_count_mismatch() {
        let embedder = test_embedder(4, 3, true);
        let error = embedder
            .embed(&input(2))
            .await
            .expect_err("count mismatch must fail");
        assert!(matches!(
            error,
            EmbeddingError::CountMismatch {
                expected: 2,
                got: 1
            }
        ));
    }

    #[test]
    fn validates_dimensions_and_hides_token_content_in_debug() {
        let error = validate_batch(1, 3, &[vec![0.0; 2]]).expect_err("wrong dimension must fail");
        assert!(matches!(
            error,
            EmbeddingError::DimensionMismatch {
                index: 0,
                expected: 3,
                got: 2
            }
        ));
        let token = TokenOnlyText::from_token_only("[EMAIL:opaque]");
        assert_eq!(format!("{token:?}"), "TokenOnlyText(<redacted>)");
    }

    #[test]
    fn metadata_builds_collection_spec_with_provenance_and_dimension() {
        let metadata = EmbeddingMetadata::new("xberg:preset:balanced", 7, 768, 32).unwrap();
        let spec = metadata.collection_spec("documents");
        assert_eq!(spec.embedding_dim, 768);
        assert_eq!(
            spec.embedding_source.as_deref(),
            Some("xberg:preset:balanced")
        );
        assert_eq!(spec.embedding_version, 7);
    }
}
