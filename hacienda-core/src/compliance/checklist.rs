use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChecklistItem {
    pub article: String,
    pub description: String,
    pub status: String,
    pub evidence: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComplianceChecklist {
    pub gdpr_articles: Vec<ChecklistItem>,
    pub ai_act_articles: Vec<ChecklistItem>,
    pub dora_articles: Vec<ChecklistItem>,
}
