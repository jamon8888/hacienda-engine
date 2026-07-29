//! hacienda — the xberg document-intelligence stack with PII, audit, and compliance.
//!
//! This crate is a distribution, not an implementation. It re-exports [`xberg`] for
//! extraction and enrichment and [`hacienda_core`] for detection, redaction, the
//! tamper-evident audit chain, human review, glossaries, and compliance artefacts.
//!
//! ```no_run
//! use hacienda::prelude::*;
//!
//! # async fn run() -> Result<(), Box<dyn std::error::Error>> {
//! let facade = HaciendaFacade::new(HaciendaConfig::default().with_pii(PipelineConfig::default()))?;
//! let result = facade.process(ExtractInput::from_uri("contract.pdf")).await?;
//! # Ok(())
//! # }
//! ```

pub use hacienda_core;
pub use xberg;

pub use hacienda_core::{audit, compliance, glossary, pii, redaction, review};
pub use hacienda_core::{HaciendaConfig, HaciendaError, HaciendaFacade, HaciendaResult};

/// Everything needed to run the default pipeline.
pub mod prelude {
    pub use crate::{HaciendaConfig, HaciendaError, HaciendaFacade, HaciendaResult};
    pub use hacienda_core::audit::{AuditChain, AuditEntry, ExportFormat};
    pub use hacienda_core::compliance::{ComplianceConfig, ComplianceReport};
    pub use hacienda_core::glossary::{EntityGlossary, GlossaryConfig};
    pub use hacienda_core::pii::{PiiPipeline, PipelineConfig, PipelineResult};
    pub use hacienda_core::redaction::{RedactionConfig, RedactionMode};
    pub use hacienda_core::review::{ReviewConfig, ReviewQueue};
    pub use xberg::{extract, extract_batch, ExtractInput, ExtractionConfig, ExtractionResult};
}
