use pii_audit::{AuditChain as PiiAuditChain, AuditEntry as PiiAuditEntry, AuditSink};
use blake3;

pub struct AuditChain {
    entries: Vec<AuditEntry>,
    last_hash: String,
    config_hash: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct AuditEntry {
    pub id: String,
    pub timestamp: String,
    pub category: String,
    pub action: RedactionAction,
    pub span_hash: String,
    pub span_length: u32,
    pub confidence: Option<f32>,
    pub source: EntitySource,
    pub pipeline_version: String,
    pub config_hash: String,
    pub chain_hash: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RedactionAction {
    Mask,
    Hash,
    Pseudonymize,
    Remove,
    Custom(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EntitySource {
    Regex,
    Model,
}

impl AuditChain {
    pub fn new(config_hash: String) -> Self {
        Self {
            entries: Vec::new(),
            last_hash: "0000000000000000000000000000000000000000000000000000000000000000".to_string(),
            config_hash,
        }
    }

    pub fn append(&mut self, entry: AuditEntry) -> Result<(), String> {
        // Validate config hash matches
        if entry.config_hash != self.config_hash {
            return Err("Config hash mismatch".into());
        }

        // Compute chain hash
        let entry_json = serde_json::to_string(&entry).map_err(|e| e.to_string())?;
        let hash_input = format!("{}{}", self.last_hash, entry_json);
        let chain_hash = blake3::hash(hash_input.as_bytes()).to_hex().to_string();

        if entry.chain_hash != chain_hash {
            return Err("Chain hash mismatch".into());
        }

        self.last_hash = chain_hash;
        self.entries.push(entry);
        Ok(())
    }

    pub fn verify(&self) -> Result<(), String> {
        let mut last_hash = "0000000000000000000000000000000000000000000000000000000000000000".to_string();
        
        for entry in &self.entries {
            let entry_json = serde_json::to_string(&entry).map_err(|e| e.to_string())?;
            let expected_hash = blake3::hash(format!("{}{}", last_hash, entry_json).as_bytes()).to_hex().to_string();
            
            if entry.chain_hash != expected_hash {
                return Err(format!("Chain integrity broken at entry {}", entry.id));
            }
            
            last_hash = entry.chain_hash.clone();
        }
        
        Ok(())
    }

    pub fn entries(&self) -> &[AuditEntry] {
        &self.entries
    }
}