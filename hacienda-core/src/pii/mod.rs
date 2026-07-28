//! PII detection: deterministic patterns, a statistical NER backend, and the merge
//! step that reconciles them.
//!
//! The two detectors are deliberately independent. [`engine::RegexEngine`] finds spans
//! with a fixed, auditable pattern set; [`ner::NerDetector`] delegates inference to an
//! `xberg` backend. [`merge::merge_entities`] resolves their overlaps, and
//! [`pipeline::PiiPipeline`] runs the whole sequence including redaction.

pub mod config;
pub mod engine;
pub mod merge;
pub mod ner;
pub mod patterns;
pub mod pipeline;
pub mod types;

pub use config::{CliOverrides, ModelConfig, PipelineConfig};
pub use engine::RegexEngine;
pub use merge::{merge_entities, MergedEntity};
pub use ner::NerDetector;
pub use patterns::builtin_patterns;
pub use pipeline::{PiiPipeline, PipelineMetrics, PipelineResult};
pub use types::{
    EntitySource, MergeConfig, MergePriority, ModelEntity, PatternMeta, PiiCategory, RegexEntity,
};

use thiserror::Error;

/// Every way the detection pipeline can fail.
#[derive(Debug, Error)]
pub enum PiiError {
    /// A detection pattern did not compile. Carries the category so a bad
    /// caller-supplied pattern can be identified without re-running the set.
    #[error("pattern for category '{category}' failed to compile")]
    Pattern {
        category: String,
        #[source]
        source: regex::Error,
    },

    /// The NER backend failed to load or to run.
    ///
    /// Boxed because `xberg::XbergError` is large relative to the other variants.
    #[error("{message}")]
    Ner {
        message: String,
        #[source]
        source: Box<xberg::XbergError>,
    },

    /// The configuration asks for a model the build cannot provide.
    ///
    /// Loading a model is feature-gated, so a config that enables one is a hard
    /// error rather than a silent downgrade to regex-only detection — under-detecting
    /// PII without saying so is the worst possible failure mode here.
    #[error("model backend unavailable: {0}")]
    ModelUnavailable(String),

    #[error("reading configuration from {path}")]
    ConfigIo {
        path: String,
        #[source]
        source: std::io::Error,
    },

    #[error("parsing configuration from {path}")]
    ConfigParse {
        path: String,
        #[source]
        source: toml::de::Error,
    },

    #[error(transparent)]
    Redaction(#[from] crate::redaction::RedactionError),
}
