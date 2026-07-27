mod generator;
mod dpia;
mod model_card;
mod dora;
mod checklist;

pub use generator::ComplianceGenerator;
pub use dpia::DpiaDocument;
pub use model_card::{generate_model_card, ModelCard};
pub use dora::{DoraReport, PiiIncident};
pub use checklist::{ComplianceChecklist, ChecklistItem};

pub struct ComplianceGenerator {
    model_name: String,
    dpia_generator: DpiaGenerator,
}

impl ComplianceGenerator {
    pub fn new(model_name: &str) -> Self {
        Self {
            model_name: model_name.to_string(),
            dpia_generator: DpiaGenerator::new(model_name),
        }
    }

    pub async fn full_report(&self) -> ComplianceReport {
        let dpia = self.dpia_generator.generate();
        let model_card = generate_model_card(&self.model_name);
        let dora = crate::dora::generate_report(&PiiIncident {
            summary: "No incident".into(),
            timeline: "N/A".into(),
            root_cause: "N/A".into(),
            detected_at: chrono::Utc::now().to_rfc3339(),
            contained_at: None,
            resolved_at: None,
            actions_taken: vec![],
            lessons_learned: vec![],
        });
        ComplianceReport { dpia, model_card, dora, generated_at: chrono::Utc::now().to_rfc3339() }
    }

    pub async fn model_card(&self) -> ModelCard {
        generate_model_card(&self.model_name)
    }

    pub async fn dpia(&self) -> DpiaDocument {
        self.dpia_generator.generate()
    }

    pub async fn dora_report(&self, incident: &PiiIncident) -> DoraReport {
        crate::dora::generate_report(incident)
    }

    pub async fn checklist(&self) -> ComplianceChecklist {
        ComplianceChecklist {
            gdpr_articles: vec![],
            ai_act_articles: vec![],
            dora_articles: vec![],
        }
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ComplianceReport {
    pub dpia: DpiaDocument,
    pub model_card: ModelCard,
    pub dora: DoraReport,
    pub generated_at: String,
}