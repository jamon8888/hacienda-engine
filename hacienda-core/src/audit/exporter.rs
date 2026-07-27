use crate::audit::{AuditChain, AuditEntry, AuditSink};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExportFormat {
    Json,
    JsonLines,
    Csv,
}

pub struct AuditExporter;

impl AuditExporter {
    pub fn export(
        chain: &crate::audit::AuditChain,
        format: ExportFormat,
    ) -> Result<String, String> {
        let entries = chain.entries();
        
        match format {
            ExportFormat::Json => {
                serde_json::to_string_pretty(entries)
                    .map_err(|e| e.to_string())
            }
            ExportFormat::JsonLines => {
                let lines: Vec<String> = entries.iter()
                    .map(|e| serde_json::to_string(e).unwrap())
                    .collect();
                Ok(lines.join("\n"))
            }
            ExportFormat::Csv => {
                let mut wtr = csv::Writer::from_writer(vec![]);
                for entry in entries {
                    wtr.serialize(entry).map_err(|e| e.to_string())?;
                }
                String::from_utf8(wtr.into_inner().map_err(|e| e.to_string())?)
                    .map_err(|e| e.to_string())
            }
        }
    }
}