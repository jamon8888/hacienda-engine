use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
/// PiiIncident struct
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
/// DoraReport struct
pub struct DoraReport {
    pub reference: String,
    pub timestamp: String,
    pub entity: String,
    pub classification: IncidentClassification,
    pub description: IncidentDescription,
    pub response: IncidentResponse,
    pub communication: CommunicationLog,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
/// IncidentClassification struct
pub struct IncidentClassification {
    pub severity: String,
    pub category: String,
    pub impact: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
/// IncidentDescription struct
pub struct IncidentDescription {
    pub summary: String,
    pub timeline: String,
    pub root_cause: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
/// IncidentResponse struct
pub struct IncidentResponse {
    pub detected_at: String,
    pub contained_at: Option<String>,
    pub resolved_at: Option<String>,
    pub actions_taken: Vec<String>,
    pub lessons_learned: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
/// CommunicationLog struct
pub struct CommunicationLog {
    pub internal_notifications: Vec<String>,
    pub regulatory_notifications: Vec<String>,
    pub public_communications: Vec<String>,
}

/// DoraIncidentReporter struct
pub struct DoraIncidentReporter;

impl DoraIncidentReporter {
/// generate_report function
    pub fn generate_report(incident: &PiiIncident) -> DoraReport {
        let sanitized = incident.detected_at.replace(['-', ':', 'T'], "");
        let timestamp_part = sanitized.get(..13).unwrap_or(&sanitized);
        let reference = format!("PII-INC-{}", timestamp_part);

        DoraReport {
            reference,
            timestamp: incident.detected_at.clone(),
            entity: "Xberg PII Processing Platform".to_string(),
            classification: IncidentClassification {
                severity: "High".to_string(),
                category: "Data Breach - PII Exposure".to_string(),
                impact:
                    "Potential unauthorized access to personal data through PII extraction pipeline"
                        .to_string(),
            },
            description: IncidentDescription {
                summary: incident.summary.clone(),
                timeline: incident.timeline.clone(),
                root_cause: incident.root_cause.clone(),
            },
            response: IncidentResponse {
                detected_at: incident.detected_at.clone(),
                contained_at: incident.contained_at.clone(),
                resolved_at: incident.resolved_at.clone(),
                actions_taken: incident.actions_taken.clone(),
                lessons_learned: incident.lessons_learned.clone(),
            },
            communication: CommunicationLog {
                internal_notifications: vec![
                    "Security team notified immediately upon detection".to_string(),
                    "DPO briefed within 1 hour".to_string(),
                    "Executive leadership notified per incident response policy".to_string(),
                ],
                regulatory_notifications: vec![
                    "Supervisory authority notification prepared (GDPR Article 33)".to_string(),
                    "DORA incident report submitted to competent authority".to_string(),
                ],
                public_communications: vec![
                    "Affected individuals notified per GDPR Article 34 (if applicable)".to_string(),
                    "Public disclosure prepared per DORA requirements".to_string(),
                ],
            },
        }
    }
}

/// generate_report function
pub fn generate_report(incident: &PiiIncident) -> DoraReport {
    DoraIncidentReporter::generate_report(incident)
}
