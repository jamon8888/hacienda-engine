use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ModelCard {
    pub model_details: ModelDetails,
    pub training_data: TrainingData,
    pub evaluation: Evaluation,
    pub bias_fairness: BiasFairness,
    pub deployment: Deployment,
    pub governance: Governance,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ModelDetails {
    pub name: String,
    pub version: String,
    pub description: String,
    pub architecture: String,
    pub parameters: String,
    pub license: String,
    pub contact: String,
    pub date: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TrainingData {
    pub description: String,
    pub size: String,
    pub sources: Vec<String>,
    pub preprocessing: String,
    pub pii_categories: u32,
    pub languages: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Evaluation {
    pub description: String,
    pub metrics: Vec<Metric>,
    pub benchmarks: Vec<String>,
    pub limitations: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Metric {
    pub name: String,
    pub value: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BiasFairness {
    pub description: String,
    pub known_biases: Vec<String>,
    pub mitigation_strategies: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Deployment {
    pub intended_use: String,
    pub out_of_scope_uses: Vec<String>,
    pub operational_factors: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Governance {
    pub organization: String,
    pub review_process: String,
    pub update_frequency: String,
    pub changelog: Vec<String>,
}

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
            preprocessing: "Tokenization via DeBERTa tokenizer with MAX_WIDTH=8 token window limitation. Entities truncated at 8 subword tokens. Negative sampling for false-positive reduction.".to_string(),
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
                "MAX_WIDTH=8 token limitation may miss very long entity spans (e.g., full addresses in non-standard formats)".to_string(),
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
