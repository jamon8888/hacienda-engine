use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
/// Likelihood enum
pub enum Likelihood {
    /// Low variant
    Low,
    /// Medium variant
    Medium,
    /// High variant
    High,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
/// Severity enum
pub enum Severity {
    /// Low variant
    Low,
    /// Medium variant
    Medium,
    /// High variant
    High,
    /// Critical variant
    Critical,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
/// ResidualRisk enum
pub enum ResidualRisk {
    /// Negligible variant
    Negligible,
    /// Low variant
    Low,
    /// Medium variant
    Medium,
    /// High variant
    High,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
/// Risk struct
pub struct Risk {
    pub id: String,
    pub description: String,
    pub likelihood: Likelihood,
    pub severity: Severity,
    pub mitigation: String,
    pub residual_risk: ResidualRisk,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
/// Annex enum
pub enum Annex {
    /// ModelCard variant
    ModelCard(String),
    /// RiskAssessment variant
    RiskAssessment(String),
    /// DataFlowDiagram variant
    DataFlowDiagram(String),
    /// SecurityMeasures variant
    SecurityMeasures(String),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
/// DpiaDocument struct
pub struct DpiaDocument {
    pub processing_description: String,
    pub necessity_proportionality: String,
    pub risks: Vec<Risk>,
    pub mitigation_measures: Vec<String>,
    pub dpo_opinion_template: String,
    pub annexes: Vec<Annex>,
}

/// DpiaGenerator struct
pub struct DpiaGenerator {
    model_name: String,
}

impl DpiaGenerator {
/// new function
    pub fn new(model_name: &str) -> Self {
        Self {
            model_name: model_name.to_string(),
        }
    }

/// generate function
    pub fn generate(&self) -> DpiaDocument {
        DpiaDocument {
            processing_description: format!(
                "Data Protection Impact Assessment for PII extraction pipeline using model '{}'. \
                 The processing involves automated detection, classification, and \
                 redaction of personally identifiable information from documents across multiple \
                 formats including PDF, images, spreadsheets, and email archives.",
                self.model_name
            ),
            necessity_proportionality: "The processing of PII is necessary for the legitimate \
                purpose of compliance enforcement and document redaction. The scope is limited to \
                extracting and redacting personal data. Less intrusive alternatives (e.g., manual \
                review) are not feasible at scale. Data minimization is enforced through \
                configurable extraction policies and retention limits."
                .to_string(),
            risks: vec![
                Risk {
                    id: "R001".to_string(),
                    description: "False positive PII detection leading to legitimate content being redacted or flagged incorrectly"
                        .to_string(),
                    likelihood: Likelihood::Medium,
                    severity: Severity::Medium,
                    mitigation: "Confidence thresholds, human-in-the-loop review for low-confidence detections, continuous model evaluation against benchmark datasets"
                        .to_string(),
                    residual_risk: ResidualRisk::Low,
                },
                Risk {
                    id: "R002".to_string(),
                    description: "False negative PII detection allowing personal data to pass through unredacted"
                        .to_string(),
                    likelihood: Likelihood::Medium,
                    severity: Severity::High,
                    mitigation: "Ensemble detection with multiple models, regular retraining on updated PII patterns, mandatory audit logging of all extraction results"
                        .to_string(),
                    residual_risk: ResidualRisk::Low,
                },
                Risk {
                    id: "R003".to_string(),
                    description: "PII exposure through audit logs containing extracted personal data"
                        .to_string(),
                    likelihood: Likelihood::Low,
                    severity: Severity::High,
                    mitigation: "Audit log encryption at rest, automatic log rotation and retention policies, redaction of PII from structured audit entries, access controls on audit infrastructure"
                        .to_string(),
                    residual_risk: ResidualRisk::Negligible,
                },
                Risk {
                    id: "R004".to_string(),
                    description: "Model bias causing discriminatory PII detection across demographic groups or languages"
                        .to_string(),
                    likelihood: Likelihood::Medium,
                    severity: Severity::High,
                    mitigation: "Multi-language training data covering diverse name patterns, bias testing across demographic groups, transparency reporting of model performance metrics per category"
                        .to_string(),
                    residual_risk: ResidualRisk::Low,
                },
            ],
            mitigation_measures: vec![
                "Encryption of PII at rest and in transit (AES-256-GCM)".to_string(),
                "Access controls with role-based permissions for pipeline operators".to_string(),
                "Automatic data retention and deletion policies".to_string(),
                "Audit logging with integrity chain verification".to_string(),
                "Regular penetration testing and security assessments".to_string(),
                "Incident response procedures aligned with DORA requirements".to_string(),
                "Data processing agreements with all third-party processors".to_string(),
                "Annual DPIA review and update cycle".to_string(),
            ],
            dpo_opinion_template: "I have reviewed this Data Protection Impact Assessment for the \
                PII extraction pipeline. The processing activities described are consistent with \
                the organization's data protection obligations under GDPR Article 35. The risks \
                identified are adequately mitigated by the proposed measures. I recommend proceeding \
                with the processing subject to the following conditions:\n\n\
                1. Quarterly review of false positive/negative rates\n\
                2. Annual re-training of detection models\n\
                3. Monthly audit log access reviews\n\
                4. Immediate notification of any data breaches per DORA requirements\n\n\
                DPO Signature: _______________________\n\
                Date: _______________________"
                .to_string(),
            annexes: vec![
                Annex::ModelCard(format!(
                    "Model card for {} - see model_card module",
                    self.model_name
                )),
                Annex::RiskAssessment("Detailed risk assessment matrix with likelihood × impact scoring"
                    .to_string()),
                Annex::DataFlowDiagram("Data flow diagram: Input → MIME Detection → Extraction → PII Detection → Redaction → Output"
                    .to_string()),
                Annex::SecurityMeasures("Technical and organizational security measures documentation"
                    .to_string()),
            ],
        }
    }
}
