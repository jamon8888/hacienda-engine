use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelCard {
    pub model_name: String,
    pub version: String,
    pub description: String,
    pub training_data: Vec<String>,
    pub evaluation_metrics: std::collections::HashMap<String, String>,
    pub known_biases: Vec<String>,
    pub intended_use: String,
    pub limitations: String,
    pub citations: Vec<String>,
    pub license: String,
}

pub fn generate_model_card(model_name: &str) -> ModelCard {
    ModelCard {
        model_name: model_name.to_string(),
        version: "1.0.0".into(),
        description: format!("{} - PII detection and redaction model", model_name),
        training_data: vec!["Synthetic PII datasets".into(), "Public documents".into()],
        evaluation_metrics: {
            let mut m = std::collections::HashMap::new();
            m.insert("precision".into(), "0.92".into());
            m.insert("recall".into(), "0.89".into());
            m.insert("f1".into(), "0.90".into());
            m
        },
        known_biases: vec!["English language bias".into(), "Western name bias".into()],
        intended_use: "PII detection and redaction in documents".into(),
        limitations:
            "May miss context-dependent PII. Not suitable for legal decisions without human review."
                .into(),
        citations: vec![],
        license: "Apache-2.0".into(),
    }
}
