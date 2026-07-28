//! Glossary generation settings.

use serde::{Deserialize, Serialize};

/// Markup used when linking a mention back to its glossary entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LinkStyle {
    #[default]
    Markdown,
    Html,
    Wiki,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct GlossaryConfig {
    pub enabled: bool,
    pub link_style: LinkStyle,
    /// Mentions detected below this confidence are not recorded.
    pub min_confidence: f32,
    /// Terms seen fewer than this many times are not published or linked.
    ///
    /// A glossary of one-off mentions is noise, and every published term is one more
    /// place a real name can resurface in a document that was supposed to be redacted.
    pub min_count: usize,
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
