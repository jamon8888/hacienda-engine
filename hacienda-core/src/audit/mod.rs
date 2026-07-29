//! Tamper-evident audit logging for PII operations.
//!
//! Every redaction produces an [`AuditEntry`] whose `chain_hash` covers the previous
//! entry's hash, so any edit, reorder, or deletion is detectable by [`AuditChain::verify`].
//! Original spans are never stored — only their blake3 digests.

pub mod chain;
pub mod entry;
pub mod error;
pub mod export;
pub mod segment;
pub mod sink;
pub mod store;
pub mod store_file;

pub use chain::{AuditChain, GENESIS_HASH};
pub use entry::{compute_chain_hash, AuditEntry, AuditEntryInput, EntitySource, RedactionAction};
pub use error::AuditError;
pub use export::{export, export_csv, export_json, export_json_lines, ExportFormat};
pub use segment::{compute_seal_hash, verify_seal_chain, NodeId, Segment, SegmentSeal};
#[allow(deprecated)]
pub use sink::{AuditSink, FileSink};
pub use store::{AuditStore, InMemoryAuditStore};
pub use store_file::{FileAuditStore, SyncPolicy};

use serde::{Deserialize, Serialize};

/// Settings that govern the audit chain.
///
/// # Why this struct is so small
///
/// It used to carry `log_path`, `format`, `max_files`, `max_file_size_mb`, and
/// `include_span_hash`. Nothing read any of them. An operator who set `log_path` and
/// enabled auditing got no file, no warning, and no way to tell the difference from a
/// working configuration — the exact "why is this not redacting" failure shape the CLI
/// spec (§6.3) names as the most common incident class.
///
/// They were not merely unread but wrong-shaped for what replaced them:
/// [`FileAuditStore`] writes a *directory* of segments rather than one rotating file,
/// rotation is caller-driven rather than size-triggered, and the store always records
/// span hashes because an entry without one identifies nothing. The remaining fields are
/// the two that have consumers: `enabled` gates store construction in
/// [`HaciendaFacade::build`], and `config_hash` is checked by [`AuditChain::append`].
///
/// A durable store is selected by handing one to
/// [`HaciendaFacade::with_stores`](crate::HaciendaFacade::with_stores). When the CLI
/// grows `--audit-out` it will gain a config-file counterpart here — added at the point
/// something reads it, not before.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct AuditConfig {
    /// Whether to record an audit chain at all. Read by the facade when choosing
    /// whether to build a default store.
    pub enabled: bool,
    /// Hash of the effective pipeline configuration; entries from a different
    /// configuration are refused by [`AuditChain::append`].
    pub config_hash: String,
}

impl Default for AuditConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            config_hash: "default".into(),
        }
    }
}
