use crate::compliance::PiiIncident;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DoraReport {
    pub incident_id: String,
    pub summary: String,
    pub timeline: String,
    pub root_cause: String,
    pub detected_at: String,
    pub contained_at: Option<String>,
    pub resolved_at: Option<String>,
    pub actions_taken: Vec<String>,
    pub lessons_learned: Vec<String>,
    pub severity: String,
    pub classification: String,
}

pub fn generate_report(incident: &PiiIncident) -> DoraReport {
    DoraReport {
        incident_id: uuid::Uuid::new_v4().to_string(),
        summary: incident.summary.clone(),
        timeline: incident.timeline.clone(),
        root_cause: incident.root_cause.clone(),
        detected_at: incident.detected_at.clone(),
        contained_at: incident.contained_at.clone(),
        resolved_at: incident.resolved_at.clone(),
        actions_taken: incident.actions_taken.clone(),
        lessons_learned: incident.lessons_learned.clone(),
        severity: "High".into(),
        classification: "Confidentiality breach".into(),
    }
}
