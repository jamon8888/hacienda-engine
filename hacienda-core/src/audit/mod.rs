//! Tamper-evident audit logging for PII operations.
//!
//! Every redaction produces an [`AuditEntry`] whose `chain_hash` covers the previous
//! entry's hash, so any edit, reorder, or deletion is detectable by [`AuditChain::verify`].
//! Original spans are never stored — only their blake3 digests.

pub mod chain;
pub mod entry;
pub mod error;
pub mod export;
pub mod sink;

pub use chain::{AuditChain, GENESIS_HASH};
pub use entry::{compute_chain_hash, AuditEntry, AuditEntryInput, EntitySource, RedactionAction};
pub use error::AuditError;
pub use export::{export, export_csv, export_json, export_json_lines, ExportFormat};
pub use sink::{AuditSink, FileSink};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditConfig {
    pub enabled: bool,
    pub log_path: String,
    pub format: String,
    pub max_files: u32,
    pub max_file_size_mb: u64,
    pub include_span_hash: bool,
    /// Hash of the effective pipeline configuration; entries from a different
    /// configuration are refused by [`AuditChain::append`].
    pub config_hash: String,
}

impl Default for AuditConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            log_path: "audit.log".into(),
            format: "jsonl".into(),
            max_files: 30,
            max_file_size_mb: 100,
            include_span_hash: true,
            config_hash: "default".into(),
        }
    }
}

impl AuditConfig {
    pub fn max_file_size_bytes(&self) -> u64 {
        self.max_file_size_mb.saturating_mul(1024 * 1024)
    }
}
