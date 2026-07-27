use crate::redaction::RedactionConfig;

#[derive(Debug, Clone)]
pub struct RedactionProfileImpl {
    pub name: String,
    pub config: RedactionConfig,
}

impl RedactionProfileImpl {
    pub fn default() -> Self {
        Self {
            name: "default".into(),
            config: RedactionConfig::default(),
        }
    }

    pub fn pci() -> Self {
        Self {
            name: "PCI-DSS".into(),
            config: RedactionConfig {
                mode: "pseudonymize".into(),
                fpe_key: None,
                custom_patterns: vec![
                    // Credit card patterns
                    r"\b\d{4}[-\s]?\d{4}[-\s]?\d{4}[-\s]?\d{4}\b".into(),
                ],
            },
        }
    }

    pub fn hipaa() -> Self {
        Self {
            name: "HIPAA".into(),
            config: RedactionConfig {
                mode: "remove".into(),
                fpe_key: None,
                custom_patterns: vec![
                    // SSN
                    r"\b\d{3}-\d{2}-\d{4}\b".into(),
                    // MRN
                    r"\bMRN[:\s]*\d+\b".into(),
                ],
            },
        }
    }

    pub fn gdpr() -> Self {
        Self {
            name: "GDPR".into(),
            config: RedactionConfig {
                mode: "pseudonymize".into(),
                fpe_key: None,
                custom_patterns: vec![
                    // Email
                    r"\b[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Z|a-z]{2,}\b".into(),
                    // Phone
                    r"\b\+?\d{1,3}[-.\s]?\(?\d{1,4}\)?[-.\s]?\d{1,4}[-.\s]?\d{1,9}\b".into(),
                ],
            },
        }
    }
}

impl Default for RedactionProfileImpl {
    fn default() -> Self {
        Self::default()
    }
}