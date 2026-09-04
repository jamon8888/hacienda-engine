use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
/// ModelCard struct
pub struct ModelCard {
    /// model_details field
    pub model_details: ModelDetails,
    /// training_data field
    pub training_data: TrainingData,
    /// evaluation field
    pub evaluation: Evaluation,
    /// bias_fairness field
    pub bias_fairness: BiasFairness,
    /// deployment field
    pub deployment: Deployment,
    /// governance field
    pub governance: Governance,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
/// ModelDetails struct
pub struct ModelDetails {
    /// name field
    pub name: String,
    /// version field
    pub version: String,
    /// description field
    pub description: String,
    /// architecture field
    pub architecture: String,
    /// parameters field
    pub parameters: String,
    /// license field
    pub license: String,
    /// contact field
    pub contact: String,
    /// date field
    pub date: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
/// TrainingData struct
pub struct TrainingData {
    /// description field
    pub description: String,
    /// size field
    pub size: String,
    /// sources field
    pub sources: Vec<String>,
    /// preprocessing field
    pub preprocessing: String,
    /// pii_categories field
    pub pii_categories: u32,
    /// languages field
    pub languages: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
/// Evaluation struct
pub struct Evaluation {
    /// description field
    pub description: String,
    /// metrics field
    pub metrics: Vec<Metric>,
    /// benchmarks field
    pub benchmarks: Vec<String>,
    /// limitations field
    pub limitations: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
/// Metric struct
pub struct Metric {
    /// name field
    pub name: String,
    /// value field
    pub value: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
/// BiasFairness struct
pub struct BiasFairness {
    /// description field
    pub description: String,
    /// known_biases field
    pub known_biases: Vec<String>,
    /// mitigation_strategies field
    pub mitigation_strategies: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
/// Deployment struct
pub struct Deployment {
    /// intended_use field
    pub intended_use: String,
    /// out_of_scope_uses field
    pub out_of_scope_uses: Vec<String>,
    /// operational_factors field
    pub operational_factors: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
/// Governance struct
pub struct Governance {
    /// organization field
    pub organization: String,
    /// review_process field
    pub review_process: String,
    /// update_frequency field
    pub update_frequency: String,
    /// changelog field
    pub changelog: Vec<String>,
}

/// generate_model_card function
pub fn generate_model_card(model_name: &str) -> ModelCard {
    ModelCard {
        model_details: ModelDetails {
            name: model_name.to_string(),
            version: "1.0.0".to_string(),
            description: "Multi-PII entity extraction model for document redaction and compliance workflows. Trained for high-precision detection of personally identifiable information across multiple languages and document types."
                .to_string(),
            architecture: "DeBERTa-v2 with token classification head. Fine-tuned for NER tasks with special handling for nested and overlapping entity spans.".to_string(),
            parameters: "300M".to_string(),
            license: "Apache-2.0".to_string(),
            contact: "pii-team@xberg.io".to_string(),
            date: "2025-01-01".to_string(),
        },
        training_data: TrainingData {
            description: "Curated corpus of annotated documents with PII labels across multiple languages and domains. Includes synthetic PII generation for data augmentation.".to_string(),
            size: "~500K annotated entity spans".to_string(),
            sources: vec![
                "Synthetic PII corpus (augmented)".to_string(),
                "Annotated public datasets (de-identified)".to_string(),
                "Domain-specific document collections".to_string(),
            ],
            preprocessing: "Tokenization via DeBERTa tokenizer with MAX_WIDTH=8 word window limitation. Entities truncated at 8 words. Negative sampling for false-positive reduction.".to_string(),
            pii_categories: 42,
            languages: vec![
                "English".to_string(),
                "German".to_string(),
                "French".to_string(),
                "Spanish".to_string(),
                "Italian".to_string(),
                "Portuguese".to_string(),
                "Dutch".to_string(),
            ],
        },
        evaluation: Evaluation {
            description: "Evaluated on held-out test sets spanning all supported PII categories and languages. Metrics computed at entity level (exact match).".to_string(),
            metrics: vec![
                Metric { name: "Precision".to_string(), value: "0.94".to_string() },
                Metric { name: "Recall".to_string(), value: "0.91".to_string() },
                Metric { name: "F1 Score".to_string(), value: "0.925".to_string() },
            ],
            benchmarks: vec![
                "PII-NER benchmark (multi-language)".to_string(),
                "Document redaction accuracy test suite".to_string(),
            ],
            limitations: vec![
                "MAX_WIDTH=8 word limitation may miss very long entity spans (e.g., full addresses in non-standard formats)".to_string(),
                "Lower accuracy on handwritten text or low-resolution scans".to_string(),
                "Reduced performance on code-mixed text (multiple languages in one document)".to_string(),
                "Not designed for PII detection in audio or video streams".to_string(),
            ],
        },
        bias_fairness: BiasFairness {
            description: "Bias assessment conducted across demographic groups and geographic naming patterns. Model tested for equitable PII detection regardless of name origin, address format, or cultural context."
                .to_string(),
            known_biases: vec![
                "Higher false-negative rate for non-western name formats (East Asian, South Asian naming conventions)".to_string(),
                "Reduced accuracy for non-standard address formats (rural routes, PO boxes, informal settlements)".to_string(),
                "Lower recall for PII in scripts not covered during training (Arabic, Cyrillic)".to_string(),
            ],
            mitigation_strategies: vec![
                "Ongoing data collection to improve coverage of underrepresented name patterns".to_string(),
                "Regular bias audits against demographic fairness benchmarks".to_string(),
                "Confidence thresholding to flag low-certainty detections for human review".to_string(),
            ],
        },
        deployment: Deployment {
            intended_use: "Automated PII detection and redaction in document processing pipelines. Designed for compliance with GDPR, AI Act, and DORA regulations.".to_string(),
            out_of_scope_uses: vec![
                "Surveillance or tracking of individuals".to_string(),
                "Determining creditworthiness or insurance eligibility".to_string(),
                "Law enforcement identification without warrant".to_string(),
                "Real-time video or audio PII detection".to_string(),
            ],
            operational_factors: vec![
                "Input quality: Model performs best on digital text; scanned documents may require OCR preprocessing".to_string(),
                "Batch size: Optimal throughput at 32-64 documents per batch".to_string(),
                "Latency: ~15ms per page on GPU, ~200ms on CPU".to_string(),
            ],
        },
        governance: Governance {
            organization: "Xberg PII Team".to_string(),
            review_process: "Internal peer review plus quarterly external audit. Changes to model weights require DPO sign-off per GDPR Article 35.".to_string(),
            update_frequency: "Quarterly re-training with new PII patterns. Annual comprehensive model review.".to_string(),
            changelog: vec![
                "v1.0.0 (2025-01-01): Initial release with 42 PII categories across 7 languages".to_string(),
            ],
        },
    }
}
