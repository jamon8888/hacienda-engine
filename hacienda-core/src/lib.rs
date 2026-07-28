//! PII detection, redaction, audit, review, glossary, and compliance for documents.
//!
//! The pipeline is deterministic patterns plus an optional statistical NER backend,
//! merged into one non-overlapping span list, redacted, and recorded in a
//! tamper-evident hash chain that stores digests rather than the spans themselves.
//!
//! Document extraction and inference both come from `xberg`, which this crate consumes
//! as a facade and never modifies.
//!
//! # Example
//!
//! ```no_run
//! use hacienda_core::{HaciendaConfig, HaciendaFacade};
//! use hacienda_core::pii::PipelineConfig;
//! use xberg::ExtractInput;
//!
//! # async fn run() -> Result<(), Box<dyn std::error::Error>> {
//! let config = HaciendaConfig::default().with_pii(PipelineConfig::default());
//! let facade = HaciendaFacade::new(config)?;
//!
//! let result = facade.process(ExtractInput::from_uri("contract.pdf")).await?;
//! // `content` is already redacted; the raw text never left the call.
//! println!("{}", result.extraction.results[0].content);
//! facade.verify_audit()?;
//! # Ok(())
//! # }
//! ```

pub mod audit;
pub mod compliance;
pub mod config;
pub mod error;
pub mod facade;
pub mod glossary;
pub mod pii;
pub mod redaction;
pub mod review;

pub use config::HaciendaConfig;
pub use error::HaciendaError;
pub use facade::{HaciendaFacade, HaciendaMetadata, HaciendaResult};
