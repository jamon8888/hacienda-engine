use xberg::{
    plugins::{PostProcessor, ProcessingStage, register_post_processor},
    ExtractionConfig, ExtractedDocument, Result, XbergError,
};
use hacienda_core::pii::{PiiPipelineWrapper, PipelineConfig};
use std::sync::Arc;

/// PostProcessor that runs PII pipeline as Late stage
pub struct HaciendaPiiProcessor {
    pipeline: Arc<PiiPipelineWrapper>,
}

impl HaciendaPiiProcessor {
    pub fn new(config: PipelineConfig) -> Result<Self, String> {
        Ok(Self {
            pipeline: Arc::new(PiiPipelineWrapper::new(config)?),
        })
    }
}

impl xberg::Plugin for HaciendaPiiProcessor {
    fn name(&self) -> &str { "hacienda-pii" }
    fn version(&self) -> String { env!("CARGO_PKG_VERSION").into() }
    fn description(&self) -> &str { "Hacienda PII detection & redaction" }
    fn author(&self) -> &str { "hacienda team" }
}

#[async_trait::async_trait]
impl PostProcessor for HaciendaPiiProcessor {
    fn processing_stage(&self) -> ProcessingStage { ProcessingStage::Late }
    fn priority(&self) -> i32 { 60 } // After built-in redaction (50)
    
    fn should_process(&self, doc: &ExtractedDocument, config: &ExtractionConfig) -> bool {
        doc.content.chars().any(|c: char| c.is_ascii_alphanumeric()) && 
        config.redaction.is_some()
    }
    
    async fn process(&self, doc: &mut ExtractedDocument, config: &ExtractionConfig) -> Result<()> {
        let text = &doc.content;
        if text.is_empty() { return Ok(()); }
        
        let result = self.pipeline.process(text)
            .map_err(|e| XbergError::Other(e))?;
        
        // Apply redaction to document
        doc.content = result.redacted_text.clone();
        
        // Add entities to document metadata
        let entities: Vec<xberg::types::Entity> = result.entities.iter().map(|e| {
            xberg::types::Entity {
                text: text[e.start as usize..e.end as usize].to_string(),
                category: e.category.clone(),
                confidence: e.confidence,
                start: e.start,
                end: e.end,
                source: "hacienda-pii".into(),
            }
        }).collect();
        doc.entities = Some(entities);
        
        // Add redaction report
        doc.metadata.insert("hacienda_pii_report".into(), 
            serde_json::to_value(&result).unwrap_or_default());
        
        Ok(())
    }
}

/// Register hacienda PII processor with xberg
pub fn register_hacienda_pii(config: hacienda_core::pii::PipelineConfig) -> Result<(), String> {
    let processor = HaciendaPiiProcessor::new(config)?;
    register_post_processor(Arc::new(processor))
        .map_err(|e| format!("Failed to register hacienda PII processor: {}", e))
}