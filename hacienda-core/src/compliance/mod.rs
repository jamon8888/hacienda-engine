pub mod generator;
pub mod dpia;
pub mod model_card;
pub mod dora;
pub mod ai_act;
pub mod checklist;

use pii_compliance::{ComplianceGenerator as PiiComplianceGenerator, ComplianceReport, ModelCard as PiiModelCard, DoraReport, ComplianceChecklist, ChecklistItem};

pub struct ComplianceGeneratorWrapper {
    inner: PiiComplianceGenerator,
}

impl ComplianceGeneratorWrapper {
    pub fn new(model_name: String) -> Self {
        Self { inner: PiiComplianceGenerator::new(model_name) }
    }

    pub async fn full_report(&self) -> ComplianceReport {
        self.inner.full_report().await
    }

    pub async fn model_card(&self) -> ModelCard {
        self.inner.model_card().await
    }

    pub async fn dpia(&self) -> DpiaDocument {
        self.inner.dpia().await
    }

    pub async fn dora_report(&self, incident: &PiiIncident) -> DoraReport {
        self.inner.dora_report(incident).await
    }

    pub async fn checklist(&self) -> ComplianceChecklist {
        self.inner.checklist().await
    }
}

pub struct DpiaDocument {
    pub title: String,
    pub sections: Vec<DpiaSection>,
}

pub struct DpiaSection {
    pub title: String,
    pub content: String,
}

pub struct ModelCard {
    pub model_name: String,
    pub version: String,
    pub description: String,
    pub training_data: Vec<String>,
    pub evaluation_metrics: std::collections::HashMap<String, String>,
    pub known_biases: Vec<String>,
    pub intended_use: String,
    pub limitations: String,
}

pub struct ComplianceChecklist {
    pub gdpr_articles: Vec<ChecklistItem>,
    pub ai_act_articles: Vec<ChecklistItem>,
    pub dora_articles: Vec<ChecklistItem>,
}

pub struct ChecklistItem {
    pub article: String,
    pub description: String,
    pub status: String,
    pub evidence: String,
}

pub struct DoraReport {
    pub incident_summary: String,
    pub timeline: String,
    pub root_cause: String,
    pub detected_at: String,
    pub contained_at: Option<String>,
    pub resolved_at: Option<String>,
    pub actions_taken: Vec<String>,
    pub lessons_learned: Vec<String>,
}

pub struct PiiIncident {
    pub summary: String,
    pub timeline: String,
    pub root_cause: String,
    pub detected_at: String,
    pub contained_at: Option<String>,
    pub resolved_at: Option<String>,
    pub actions_taken: Vec<String>,
    pub lessons_learned: Vec<String>,
}