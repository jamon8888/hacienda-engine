use std::collections::HashMap;
use crate::glossary::config::{GlossaryConfig, LinkStyle};
use crate::pii::PipelineEntity;

pub struct EntityGlossary {
    entries: HashMap<String, GlossaryEntry>,
    config: GlossaryConfig,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct GlossaryEntry {
    pub category: String,
    pub canonical_name: String,
    pub aliases: Vec<String>,
    pub count: usize,
    pub confidence_sum: f64,
}

impl EntityGlossary {
    pub fn new(config: GlossaryConfig) -> Self {
        Self {
            entries: HashMap::new(),
            config,
        }
    }

    pub fn insert(&mut self, entity: &PipelineEntity) {
        if entity.confidence < self.config.min_confidence {
            return;
        }

        let key = format!("{}:{}", entity.category, entity.canonical_name());
        let entry = self.entries.entry(key).or_insert_with(|| GlossaryEntry {
            category: entity.category.clone(),
            canonical_name: entity.canonical_name(),
            aliases: vec![],
            count: 0,
            confidence_sum: 0.0,
        });

        entry.count += 1;
        entry.confidence_sum += entity.confidence as f64;
    }

    pub fn generate_links(&self, text: &str) -> Result<String, String> {
        let mut result = text.to_string();
        let mut offset = 0;

        for entry in self.entries.values() {
            if entry.count < self.config.min_count {
                continue;
            }

            let avg_confidence = entry.confidence_sum / entry.count as f64;
            if avg_confidence < self.config.min_confidence {
                continue;
            }

            // Find occurrences of canonical name or aliases
            for term in std::iter::once(&entry.canonical_name).chain(entry.aliases.iter()) {
                let mut start = 0;
                while let Some(pos) = result[start..].find(term) {
                    let absolute_pos = start + pos;
                    let end_pos = absolute_pos + term.len();

                    // Check if already linked
                    let before_linked = result[..absolute_pos].contains('](');
                    let after_linked = result[end_pos..].contains('](');

                    if !before_linked && !after_linked {
                        let link = match self.config.link_style {
                            crate::glossary::config::LinkStyle::Markdown => {
                                format!("[{}](#glossary-{})", term, entry.category.to_lowercase().replace(' ', "-"))
                            }
                            crate::glossary::config::LinkStyle::Html => {
                                format!("<a href=\"#glossary-{}\">{}</a>", entry.category.to_lowercase().replace(' ', "-"), term)
                            }
                            crate::glossary::config::LinkStyle::Wiki => {
                                format!("[[{}]]", term)
                            }
                        };

                    result.replace_range(absolute_pos..end_pos, &link);
                    offset += link.len() - term.len();
                    start = absolute_pos + link.len();
                }
            }
        }

        Ok(result)
    }

    pub fn generate_glossary_section(&self) -> String {
        let mut result = String::from("## Glossary\n\n");
        
        for entry in self.entries.values() {
            if entry.count < 2 {
                continue;
            }
            
            let avg_conf = entry.confidence_sum / entry.count as f64;
            if avg_conf < 0.5 {
                continue;
            }

            result.push_str(&format!(
                "### {}\n\n**Category**: {}  \n**Mentions**: {}  \n**Avg Confidence**: {:.2}\n\n",
                entry.canonical_name,
                entry.category,
                entry.count,
                entry.confidence_sum / entry.count as f64
            ));
        }

        result
    }
}