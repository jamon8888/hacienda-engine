use crate::error::{EmailError, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Email plugin configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmailConfig {
    /// Backend to use: "imap" | "maildir" | "jmap" | "gmail" | "msgraph"
    pub backend: EmailBackend,

    /// Account name (matches config.toml account section)
    pub account: String,

    /// Archive output path (for Maildir/local storage)
    pub archive_path: Option<String>,

    /// NER model configuration
    pub ner: NerConfig,

    /// RAG indexing configuration
    pub rag: RagConfig,

    /// Redaction rules
    pub redaction: RedactionConfig,
}

impl Default for EmailConfig {
    fn default() -> Self {
        Self {
            backend: EmailBackend::Imap {
                server: "imaps://imap.example.com".to_string(),
                username: "user@example.com".to_string(),
            },
            account: "default".to_string(),
            archive_path: None,
            ner: NerConfig::default(),
            rag: RagConfig::default(),
            redaction: RedactionConfig::default(),
        }
    }
}

/// Backend configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum EmailBackend {
    #[serde(rename = "imap")]
    Imap { server: String, username: String },

    #[serde(rename = "maildir")]
    Maildir { path: String },

    #[serde(rename = "jmap")]
    Jmap { server: String, token: String },

    #[serde(rename = "gmail")]
    Gmail { client_id: String, client_secret: String },

    #[serde(rename = "msgraph")]
    Msgraph { tenant_id: String, client_id: String },
}

/// NER model configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NerConfig {
    /// Model name (e.g., "gliner-multi-v2.1")
    pub model: String,

    /// Confidence threshold for entity extraction (default: 0.5)
    pub confidence_threshold: f32,

    /// Entity types to extract (default: standard NER types)
    pub entity_types: Vec<String>,

    /// Batch size for processing (default: 32)
    pub batch_size: usize,
}

impl Default for NerConfig {
    fn default() -> Self {
        Self {
            model: "gliner-multi-v2.1".to_string(),
            confidence_threshold: 0.5,
            entity_types: vec![
                "PERSON".to_string(),
                "ORG".to_string(),
                "EMAIL".to_string(),
                "PHONE".to_string(),
                "LOC".to_string(),
                "DATE".to_string(),
                "MONEY".to_string(),
            ],
            batch_size: 32,
        }
    }
}

/// RAG indexing configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RagConfig {
    /// Chunk size for text splitting (default: 512)
    pub chunk_size: usize,

    /// Chunk overlap for context preservation (default: 50)
    pub chunk_overlap: usize,

    /// Embedding model name (default: "all-MiniLM-L6-v2")
    pub embedding_model: String,

    /// Vector index name (default: "email-corpus")
    pub index_name: String,
}

impl Default for RagConfig {
    fn default() -> Self {
        Self {
            chunk_size: 512,
            chunk_overlap: 50,
            embedding_model: "all-MiniLM-L6-v2".to_string(),
            index_name: "email-corpus".to_string(),
        }
    }
}

/// Redaction configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RedactionConfig {
    /// Rules to apply for redaction
    pub rules: Vec<RedactionRule>,

    /// Replacement string for redacted content (default: "[REDACTED]")
    pub replacement: String,

    /// Preserve format of redacted content (default: true)
    pub preserve_format: bool,
}

impl Default for RedactionConfig {
    fn default() -> Self {
        Self {
            rules: vec![
                RedactionRule::Email,
                RedactionRule::Phone,
                RedactionRule::Ssn,
                RedactionRule::CreditCard,
                RedactionRule::Iban,
            ],
            replacement: "[REDACTED]".to_string(),
            preserve_format: true,
        }
    }
}

/// Redaction rule types
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RedactionRule {
    #[serde(rename = "email")]
    Email,
    #[serde(rename = "phone")]
    Phone,
    #[serde(rename = "ssn")]
    Ssn,
    #[serde(rename = "credit_card")]
    CreditCard,
    #[serde(rename = "iban")]
    Iban,
    #[serde(rename = "ip")]
    Ip,
    #[serde(rename = "custom")]
    Custom { pattern: String, label: String },
}

/// Load configuration from TOML file
pub fn load_config(path: &str) -> Result<EmailConfig> {
    let content = std::fs::read_to_string(path)
        .map_err(|e| EmailError::Config(format!("Failed to read config file: {}", e)))?;

    let config: EmailConfig = toml::from_str(&content)
        .map_err(|e| EmailError::Config(format!("Failed to parse config: {}", e)))?;

    Ok(config)
}

/// Load configuration with secrets from environment
pub fn load_config_with_secrets(path: &str) -> Result<EmailConfig> {
    let mut config = load_config(path)?;

    // Override secrets from environment variables
    match &mut config.backend {
        EmailBackend::Imap { username, server: _ } => {
            if let Ok(pass) = std::env::var(format!("HACIENDA_EMAIL_{}_PASSWORD", config.account.to_uppercase())) {
                // Store password securely - in practice use pimalaya-config secrets
            }
        }
        EmailBackend::Jmap { token, .. } => {
            if let Ok(t) = std::env::var(format!("HACIENDA_EMAIL_{}_TOKEN", config.account.to_uppercase())) {
                *token = t;
            }
        }
        EmailBackend::Gmail { client_secret, .. } => {
            if let Ok(s) = std::env::var(format!("HACIENDA_EMAIL_{}_CLIENT_SECRET", config.account.to_uppercase())) {
                *client_secret = s;
            }
        }
        EmailBackend::Msgraph { client_id, .. } => {
            if let Ok(id) = std::env::var(format!("HACIENDA_EMAIL_{}_CLIENT_ID", config.account.to_uppercase())) {
                *client_id = id;
            }
        }
        _ => {}
    }

    Ok(config)
}
