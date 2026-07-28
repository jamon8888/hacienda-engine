//! End-to-end detection: regex + NER, merged, then redacted.
//!
//! The model half is live. The `pii-pipeline` this replaces hard-coded
//! `let model_entities = Vec::new();`, so only regex spans were ever detected and the
//! configured model was never consulted.

use crate::pii::config::PipelineConfig;
use crate::pii::engine::RegexEngine;
use crate::pii::merge::{merge_entities, MergedEntity};
use crate::pii::ner::NerDetector;
use crate::pii::types::{MergeConfig, MergePriority};
use crate::pii::PiiError;
use crate::redaction::{RedactionAuditEntry, RedactionEngine};
use serde::{Deserialize, Serialize};
use std::time::Instant;

/// Detection and redaction for one configuration.
pub struct PiiPipeline {
    regex_engine: RegexEngine,
    ner_detector: Option<NerDetector>,
    merge_config: MergeConfig,
    redaction_engine: RedactionEngine,
    config: PipelineConfig,
}

/// Everything one `process` call produced.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PipelineResult {
    /// Input text with every merged span rewritten. Equal to the input for [`PiiPipeline::scan`].
    pub redacted_text: String,
    pub entities: Vec<MergedEntity>,
    pub audit_log: Vec<RedactionAuditEntry>,
    pub metrics: PipelineMetrics,
}

/// Per-stage timings and counts for one `process` call.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PipelineMetrics {
    pub regex_ms: u64,
    pub model_ms: u64,
    pub merge_ms: u64,
    pub redaction_ms: u64,
    pub total_ms: u64,
    pub entities_detected: u32,
    pub entities_redacted: u32,
}

impl PiiPipeline {
    /// Build a pipeline, loading the NER backend named by the configuration.
    ///
    /// # Errors
    ///
    /// Returns [`PiiError::Pattern`] if a built-in pattern fails to compile,
    /// [`PiiError::Ner`] if the model cannot be loaded, and
    /// [`PiiError::ModelUnavailable`] if the configuration enables a model this
    /// build cannot load.
    pub fn new(config: PipelineConfig) -> Result<Self, PiiError> {
        let detector = if config.model.enabled {
            Some(load_detector(&config)?)
        } else {
            None
        };
        Self::assemble(config, detector)
    }

    /// Build a pipeline around a caller-supplied detector.
    ///
    /// This is the entry point for backends that are not loaded from a local model
    /// directory — a WASM bridge, a remote service, or a test double.
    ///
    /// # Errors
    ///
    /// Returns [`PiiError::Pattern`] if a built-in pattern fails to compile.
    pub fn with_detector(
        config: PipelineConfig,
        detector: Option<NerDetector>,
    ) -> Result<Self, PiiError> {
        Self::assemble(config, detector)
    }

    fn assemble(
        config: PipelineConfig,
        ner_detector: Option<NerDetector>,
    ) -> Result<Self, PiiError> {
        let merge_config = MergeConfig {
            overlap_threshold: config.merge_overlap_threshold,
            priority: if config.regex_first {
                MergePriority::RegexFirst
            } else {
                MergePriority::HigherConfidence
            },
            ..Default::default()
        };

        Ok(Self {
            regex_engine: RegexEngine::new()?,
            ner_detector,
            merge_config,
            redaction_engine: RedactionEngine::new(config.redaction.clone()),
            config,
        })
    }

    pub fn config(&self) -> &PipelineConfig {
        &self.config
    }

    /// True when a NER backend is loaded. A `false` here means regex-only detection.
    pub fn has_model(&self) -> bool {
        self.ner_detector.is_some()
    }

    /// Detect and redact.
    ///
    /// # Errors
    ///
    /// Returns [`PiiError::Ner`] if the NER backend fails. Detection failures are not
    /// swallowed: returning partially-detected text as if it were redacted would leak PII.
    pub async fn process(&self, text: &str) -> Result<PipelineResult, PiiError> {
        let start = Instant::now();
        let (entities, mut metrics) = self.detect(text).await?;

        let redaction_start = Instant::now();
        let redaction = self.redaction_engine.redact(text, &entities);
        metrics.redaction_ms = redaction_start.elapsed().as_millis() as u64;
        metrics.entities_redacted = redaction.metrics.entities_redacted;
        metrics.total_ms = start.elapsed().as_millis() as u64;

        Ok(PipelineResult {
            redacted_text: redaction.text,
            entities,
            audit_log: redaction.audit_log,
            metrics,
        })
    }

    /// Detect without rewriting the text.
    ///
    /// # Errors
    ///
    /// Returns [`PiiError::Ner`] if the NER backend fails.
    pub async fn scan(&self, text: &str) -> Result<PipelineResult, PiiError> {
        let start = Instant::now();
        let (entities, mut metrics) = self.detect(text).await?;
        metrics.total_ms = start.elapsed().as_millis() as u64;

        Ok(PipelineResult {
            redacted_text: text.to_string(),
            entities,
            audit_log: Vec::new(),
            metrics,
        })
    }

    async fn detect(&self, text: &str) -> Result<(Vec<MergedEntity>, PipelineMetrics), PiiError> {
        let mut metrics = PipelineMetrics::default();

        let regex_start = Instant::now();
        let regex_entities = self.regex_engine.find_all(text);
        metrics.regex_ms = regex_start.elapsed().as_millis() as u64;

        let model_start = Instant::now();
        let model_entities = match &self.ner_detector {
            Some(detector) => detector.detect(text).await?,
            None => Vec::new(),
        };
        metrics.model_ms = model_start.elapsed().as_millis() as u64;

        let merge_start = Instant::now();
        let entities = merge_entities(regex_entities, model_entities, &self.merge_config);
        metrics.merge_ms = merge_start.elapsed().as_millis() as u64;
        metrics.entities_detected = entities.len() as u32;

        Ok((entities, metrics))
    }
}

#[cfg(all(feature = "ner-candle", not(target_arch = "wasm32")))]
fn load_detector(config: &PipelineConfig) -> Result<NerDetector, PiiError> {
    let model_dir = config.model.model_dir.as_deref().ok_or_else(|| {
        PiiError::ModelUnavailable("model.enabled is true but no model_dir is set".into())
    })?;
    Ok(
        NerDetector::from_candle_local(model_dir, config.model.lora_adapter_dir.as_deref())?
            .with_threshold(config.model_threshold_default),
    )
}

#[cfg(not(all(feature = "ner-candle", not(target_arch = "wasm32"))))]
fn load_detector(_config: &PipelineConfig) -> Result<NerDetector, PiiError> {
    Err(PiiError::ModelUnavailable(
        "this build cannot load a model from disk; enable the `ner-candle` feature or supply a \
         detector via PiiPipeline::with_detector"
            .into(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pii::types::PiiCategory;
    use crate::redaction::RedactionMode;
    use std::sync::Arc;
    use xberg::text::ner::NerBackend;
    use xberg::types::entity::{Entity, EntityCategory};
    use xberg::Result as XbergResult;

    struct StubBackend(Vec<Entity>);

    #[cfg_attr(not(target_arch = "wasm32"), async_trait::async_trait)]
    #[cfg_attr(target_arch = "wasm32", async_trait::async_trait(?Send))]
    impl NerBackend for StubBackend {
        async fn detect(
            &self,
            _text: &str,
            _categories: &[EntityCategory],
        ) -> XbergResult<Vec<Entity>> {
            Ok(self.0.clone())
        }
    }

    fn person(text: &str, start: u32) -> Entity {
        Entity {
            category: EntityCategory::Person,
            text: text.into(),
            start,
            end: start + text.len() as u32,
            confidence: Some(0.95),
        }
    }

    fn config() -> PipelineConfig {
        PipelineConfig {
            redaction: crate::redaction::RedactionConfig {
                mode: RedactionMode::Mask,
                ..Default::default()
            },
            ..Default::default()
        }
    }

    fn pipeline(entities: Vec<Entity>) -> PiiPipeline {
        let detector = NerDetector::new(Arc::new(StubBackend(entities)));
        PiiPipeline::with_detector(config(), Some(detector)).unwrap()
    }

    #[tokio::test]
    async fn should_run_regex_only_when_no_detector_is_supplied() {
        let pipeline = PiiPipeline::with_detector(config(), None).unwrap();
        assert!(!pipeline.has_model());

        let result = pipeline
            .process("write to bob@example.com today")
            .await
            .unwrap();
        assert_eq!(result.entities.len(), 1);
        assert_eq!(result.entities[0].category, PiiCategory::Email);
        assert!(!result.redacted_text.contains("bob@example.com"));
    }

    #[tokio::test]
    async fn should_detect_spans_the_regex_engine_cannot_find() {
        // "Alice Martin" matches no built-in pattern; only the model finds it.
        let result = pipeline(vec![person("Alice Martin", 0)])
            .process("Alice Martin signed the lease")
            .await
            .unwrap();

        assert_eq!(result.entities.len(), 1);
        assert_eq!(result.entities[0].category, PiiCategory::Person);
        assert!(!result.redacted_text.contains("Alice Martin"));
    }

    #[tokio::test]
    async fn should_combine_regex_and_model_detections() {
        let text = "Alice Martin <alice@example.com>";
        let result = pipeline(vec![person("Alice Martin", 0)])
            .process(text)
            .await
            .unwrap();

        let categories: Vec<_> = result.entities.iter().map(|e| e.category.clone()).collect();
        assert!(categories.contains(&PiiCategory::Person));
        assert!(categories.contains(&PiiCategory::Email));
        assert!(!result.redacted_text.contains("alice@example.com"));
    }

    #[tokio::test]
    async fn should_leave_the_text_untouched_when_scanning() {
        let text = "Alice Martin <alice@example.com>";
        let result = pipeline(vec![person("Alice Martin", 0)])
            .scan(text)
            .await
            .unwrap();

        assert_eq!(result.redacted_text, text);
        assert!(result.audit_log.is_empty());
        assert_eq!(result.metrics.entities_redacted, 0);
        assert!(result.metrics.entities_detected >= 2);
    }

    #[tokio::test]
    async fn should_audit_every_redacted_span() {
        let result = pipeline(vec![person("Alice Martin", 0)])
            .process("Alice Martin <alice@example.com>")
            .await
            .unwrap();

        assert_eq!(
            result.audit_log.len() as u32,
            result.metrics.entities_redacted
        );
        // Audit records carry digests, never the original spans.
        for entry in &result.audit_log {
            assert!(!entry.span_hash.is_empty());
            assert!(!entry.span_hash.contains("Alice"));
        }
    }

    #[tokio::test]
    async fn should_return_no_entities_for_text_without_pii() {
        let result = PiiPipeline::with_detector(config(), None)
            .unwrap()
            .process("the quarterly report is attached")
            .await
            .unwrap();

        assert!(result.entities.is_empty());
        assert_eq!(result.redacted_text, "the quarterly report is attached");
    }

    #[test]
    fn should_refuse_to_build_when_a_model_is_enabled_but_none_can_be_loaded() {
        let config = PipelineConfig {
            model: crate::pii::config::ModelConfig {
                enabled: true,
                ..Default::default()
            },
            ..Default::default()
        };
        assert!(matches!(
            PiiPipeline::new(config),
            Err(PiiError::ModelUnavailable(_))
        ));
    }

    #[test]
    fn should_prefer_regex_spans_when_regex_first_is_set() {
        let pipeline = PiiPipeline::with_detector(config(), None).unwrap();
        assert_eq!(pipeline.merge_config.priority, MergePriority::RegexFirst);
    }

    #[test]
    fn should_prefer_higher_confidence_when_regex_first_is_unset() {
        let pipeline = PiiPipeline::with_detector(
            PipelineConfig {
                regex_first: false,
                ..Default::default()
            },
            None,
        )
        .unwrap();
        assert_eq!(
            pipeline.merge_config.priority,
            MergePriority::HigherConfidence
        );
    }
}
