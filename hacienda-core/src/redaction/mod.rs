pub mod engine;
pub mod fpe;
pub mod patterns;

use pii_redaction::{RedactionConfig, RedactionEngine, RedactionMode, RedactionResult};

pub struct RedactionEngineWrapper {
    inner: RedactionEngine,
}

impl RedactionEngineWrapper {
    pub fn new(config: RedactionConfig) -> Self {
        Self {
            inner: RedactionEngine::new(config),
        }
    }

    pub fn redact(&self, text: &str) -> RedactionResult {
        self.inner.redact(text)
    }
}

pub struct RedactionConfig {
    pub mode: RedactionMode,
    pub fpe_key: Option<String>,
    pub custom_template: Option<String>,
    pub preserve_format: bool,
}

pub enum RedactionMode {
    Mask,
    Hash,
    Pseudonymize,
    Remove,
    Custom,
}

#[derive(Debug)]
pub struct RedactionResult {
    pub text: String,
    pub entities: Vec<RedactionEntity>,
    pub metrics: RedactionMetrics,
}

#[derive(Debug)]
pub struct RedactionEntity {
    pub category: String,
    pub start: usize,
    pub end: usize,
    pub replacement: String,
}

#[derive(Debug)]
pub struct RedactionMetrics {
    pub entities_detected: usize,
    pub entities_redacted: usize,
}
