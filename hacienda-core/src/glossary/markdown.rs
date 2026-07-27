pub fn generate_markdown_links(text: &str, glossary: &crate::glossary::entity_linker::EntityGlossary) -> Result<String, String> {
    glossary.generate_links(text)
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct GlossaryEntry {
    pub category: String,
    pub canonical_name: String,
    pub aliases: Vec<String>,
    pub count: usize,
    pub confidence_sum: f64,
}