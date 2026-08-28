//! Entity glossary: which terms a document actually mentions, and links back to them.
//!
//! The glossary runs on detected spans, so it publishes exactly what the pipeline saw.
//! It is deliberately conservative — [`GlossaryConfig::min_count`] and
//! [`GlossaryConfig::min_confidence`] both gate publication, because every published
//! term reproduces a real mention in clear text.

/// Module config
pub mod config;
/// Module entity_linker
pub mod entity_linker;

pub use config::{GlossaryConfig, LinkStyle};
pub use entity_linker::{EntityGlossary, GlossaryEntry};
