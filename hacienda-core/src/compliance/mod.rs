//! GDPR, AI Act, and DORA compliance artefact generators.
//!
//! These generators emit documents whose *content* is derived from what the pipeline
//! actually does. They are inputs to a compliance process, not a substitute for one —
//! the DPIA still needs DPO sign-off and incident reports still need a submission.

pub mod checklist;
pub mod dora;
pub mod dpia;
pub mod model_card;

pub use checklist::{generate_checklist, ChecklistItem, ComplianceChecklist, ControlStatus};
pub use dora::{generate_report, DoraIncidentReporter, DoraReport, PiiIncident};
pub use dpia::{DpiaDocument, DpiaGenerator};
pub use model_card::{generate_model_card, ModelCard};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComplianceConfig {
    /// Model the generated artefacts describe.
    pub model_name: String,
    pub enabled_reports: Vec<ReportType>,
}

impl Default for ComplianceConfig {
    fn default() -> Self {
        Self {
            model_name: "hacienda-pii".into(),
            enabled_reports: vec![
                ReportType::Dpia,
                ReportType::ModelCard,
                ReportType::Checklist,
            ],
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReportType {
    Dpia,
    ModelCard,
    Dora,
    Checklist,
}

/// A full compliance pack for one pipeline configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComplianceReport {
    pub dpia: Option<DpiaDocument>,
    pub model_card: Option<ModelCard>,
    pub dora: Option<DoraReport>,
    pub checklist: Option<ComplianceChecklist>,
    pub generated_at: String,
}

pub struct ComplianceGenerator {
    config: ComplianceConfig,
    dpia_generator: DpiaGenerator,
}

impl ComplianceGenerator {
    pub fn new(config: ComplianceConfig) -> Self {
        let dpia_generator = DpiaGenerator::new(&config.model_name);
        Self {
            config,
            dpia_generator,
        }
    }

    /// Generate every artefact named in [`ComplianceConfig::enabled_reports`].
    ///
    /// A DORA report is only produced when an `incident` is supplied; DORA reports
    /// describe a specific event, so there is nothing meaningful to emit without one.
    pub fn report(&self, incident: Option<&PiiIncident>) -> ComplianceReport {
        let enabled = |r: ReportType| self.config.enabled_reports.contains(&r);

        ComplianceReport {
            dpia: enabled(ReportType::Dpia).then(|| self.dpia_generator.generate()),
            model_card: enabled(ReportType::ModelCard)
                .then(|| generate_model_card(&self.config.model_name)),
            dora: match (enabled(ReportType::Dora), incident) {
                (true, Some(incident)) => Some(generate_report(incident)),
                _ => None,
            },
            checklist: enabled(ReportType::Checklist).then(generate_checklist),
            generated_at: chrono::Utc::now().to_rfc3339(),
        }
    }

    pub fn model_card(&self) -> ModelCard {
        generate_model_card(&self.config.model_name)
    }

    pub fn dpia(&self) -> DpiaDocument {
        self.dpia_generator.generate()
    }

    pub fn dora_report(&self, incident: &PiiIncident) -> DoraReport {
        generate_report(incident)
    }

    pub fn checklist(&self) -> ComplianceChecklist {
        generate_checklist()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn incident() -> PiiIncident {
        PiiIncident {
            summary: "Unredacted spans reached a downstream index".into(),
            timeline: "detected 10:00, contained 10:20".into(),
            root_cause: "Model backend disabled by misconfiguration".into(),
            detected_at: "2026-07-28T10:00:00Z".into(),
            contained_at: Some("2026-07-28T10:20:00Z".into()),
            resolved_at: None,
            actions_taken: vec!["Reindexed with redaction enabled".into()],
            lessons_learned: vec!["Fail closed when the model backend is unavailable".into()],
        }
    }

    #[test]
    fn should_generate_the_default_artefacts() {
        let report = ComplianceGenerator::new(ComplianceConfig::default()).report(None);
        assert!(report.dpia.is_some());
        assert!(report.model_card.is_some());
        assert!(report.checklist.is_some());
    }

    #[test]
    fn should_omit_artefacts_that_are_not_enabled() {
        let generator = ComplianceGenerator::new(ComplianceConfig {
            model_name: "m".into(),
            enabled_reports: vec![ReportType::ModelCard],
        });
        let report = generator.report(None);
        assert!(report.model_card.is_some());
        assert!(report.dpia.is_none());
        assert!(report.checklist.is_none());
    }

    #[test]
    fn should_omit_the_dora_report_when_no_incident_is_supplied() {
        let generator = ComplianceGenerator::new(ComplianceConfig {
            model_name: "m".into(),
            enabled_reports: vec![ReportType::Dora],
        });
        assert!(generator.report(None).dora.is_none());
    }

    #[test]
    fn should_include_the_dora_report_when_an_incident_is_supplied() {
        let generator = ComplianceGenerator::new(ComplianceConfig {
            model_name: "m".into(),
            enabled_reports: vec![ReportType::Dora],
        });
        let report = generator.report(Some(&incident()));
        assert!(report.dora.unwrap().reference.starts_with("PII-INC-"));
    }

    #[test]
    fn should_name_the_configured_model_in_the_generated_artefacts() {
        let generator = ComplianceGenerator::new(ComplianceConfig {
            model_name: "gliner2-pii-fr".into(),
            ..Default::default()
        });
        assert_eq!(generator.model_card().model_details.name, "gliner2-pii-fr");
        assert!(generator
            .dpia()
            .processing_description
            .contains("gliner2-pii-fr"));
    }
}
