use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DpiaDocument {
    pub project_name: String,
    pub data_controller: String,
    pub dpo_contact: Option<String>,
    pub processing_purposes: Vec<String>,
    pub data_categories: Vec<String>,
    pub data_subjects: Vec<String>,
    pub recipients: Vec<String>,
    pub transfers: Vec<String>,
    pub retention_periods: Vec<String>,
    pub security_measures: Vec<String>,
    pub risk_assessment: RiskAssessment,
    pub mitigation_measures: Vec<String>,
    pub sign_off: SignOff,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RiskAssessment {
    pub likelihood: String,
    pub severity: String,
    pub overall_risk: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SignOff {
    pub dpo_name: Option<String>,
    pub dpo_signature: Option<String>,
    pub date: String,
}

pub struct DpiaGenerator {
    model_name: String,
}

impl DpiaGenerator {
    pub fn new(model_name: &str) -> Self {
        Self { model_name: model_name.to_string() }
    }

    pub fn generate(&self) -> DpiaDocument {
        DpiaDocument {
            project_name: format!("{} PII Processing", self.model_name),
            data_controller: "Data Controller".into(),
            dpo_contact: None,
            processing_purposes: vec!["Document analysis".into(), "PII detection".into()],
            data_categories: vec!["Personal identifiers".into(), "Contact info".into()],
            data_subjects: vec!["Document authors".into(), "Subjects in documents".into()],
            recipients: vec!["Internal systems".into()],
            transfers: vec![],
            retention_periods: vec!["30 days".into()],
            security_measures: vec!["Encryption at rest".into(), "Access controls".into()],
            risk_assessment: RiskAssessment {
                likelihood: "Medium".into(),
                severity: "High".into(),
                overall_risk: "Medium".into(),
            },
            mitigation_measures: vec!["Pseudonymization".into(), "Access logging".into()],
            sign_off: SignOff {
                dpo_name: None,
                dpo_signature: None,
                date: chrono::Utc::now().to_rfc3339(),
            },
        }
    }
}