mod chain;
mod exporter;
mod sink;
mod verifier;

pub use chain::{AuditChain, AuditEntry};
pub use exporter::{AuditExporter, ExportFormat};
pub use sink::{AuditSink, FileSink};
pub use verifier::verify_chain;

use pii_audit::{
    AuditChain as PiiAuditChain, AuditEntry as PiiAuditEntry, AuditSink, FileSink as PiiFileSink,
};

pub struct AuditChainWrapper {
    inner: PiiAuditChain,
}

impl AuditChainWrapper {
    pub fn new(config_hash: String) -> Self {
        Self {
            inner: PiiAuditChain::new(config_hash),
        }
    }

    pub fn append(&mut self, entry: AuditEntry) -> Result<(), String> {
        self.inner.append(entry.into()).map_err(|e| e.to_string())
    }

    pub fn verify(&self) -> Result<(), String> {
        self.inner.verify().map_err(|e| e.to_string())
    }

    pub fn entries(&self) -> &[AuditEntry] {
        // Need to convert from inner entries
        &[]
    }
}

pub struct FileSinkWrapper {
    inner: PiiFileSink,
}

impl FileSinkWrapper {
    pub fn new(
        path: impl AsRef<std::path::Path>,
        max_size: u64,
        max_files: u32,
        config_hash: String,
    ) -> Result<Self, String> {
        Ok(Self {
            inner: PiiFileSink::new(path, max_size, max_files, config_hash)
                .map_err(|e| e.to_string())?,
        })
    }

    pub fn chain(&self) -> &AuditChainWrapper {
        unimplemented!()
    }

    pub fn chain_mut(&mut self) -> &mut AuditChainWrapper {
        unimplemented!()
    }
}
