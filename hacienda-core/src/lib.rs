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
//! facade.verify_audit().await?;
//! # Ok(())
//! # }
//! ```

pub mod audit;
// Everything below is server/native-only: `auth` pulls in axum, `facade`/`config`/`error`
// bind the whole crate (review queue, compliance reports, the file-backed audit store)
// together, and `compliance`, `glossary`, `jobs`, `review` are unused by browser-side PII
// detection and redaction (see Track L of the 2026-07-30 hacienda program plan). WASM
// builds get `pii`, `redaction`, and `audit` (its `store_file`/`sink`/`export`
// submodules stay native-only — see `audit/mod.rs` — but the in-memory `AuditStore` and
// hash chain are wasm32-safe as of Track L3).
#[cfg(not(target_arch = "wasm32"))]
pub mod auth;
#[cfg(not(target_arch = "wasm32"))]
pub mod compliance;
#[cfg(not(target_arch = "wasm32"))]
pub mod config;
#[cfg(not(target_arch = "wasm32"))]
pub mod error;
#[cfg(not(target_arch = "wasm32"))]
pub mod facade;
#[cfg(not(target_arch = "wasm32"))]
pub mod glossary;
#[cfg(all(feature = "jobs", not(target_arch = "wasm32")))]
pub mod jobs;
pub mod pii;
pub mod redaction;
#[cfg(not(target_arch = "wasm32"))]
pub mod review;
#[cfg(all(
    any(feature = "postgres", feature = "s3"),
    not(target_arch = "wasm32")
))]
pub mod store;

#[cfg(not(target_arch = "wasm32"))]
pub use auth::{AuthContext, AuthzError, Caller, Capability, CapabilitySet};
#[cfg(not(target_arch = "wasm32"))]
pub use config::HaciendaConfig;
#[cfg(not(target_arch = "wasm32"))]
pub use error::HaciendaError;
#[cfg(not(target_arch = "wasm32"))]
pub use facade::{
    HaciendaFacade, HaciendaMetadata, HaciendaResult, SpanText, TextRedactResult, TextScanResult,
};
