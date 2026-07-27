use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GlossaryConfig {
    pub enabled: bool,
    pub link_style: LinkStyle,
    pub min_confidence: f32,
    pub min_count: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LinkStyle {
    Markdown,
    Html,
    Wiki,
}

impl Default for GlossaryConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            link_style: LinkStyle::Markdown,
            min_confidence: 0.5,
            min_count: 2,
        }
    }
}