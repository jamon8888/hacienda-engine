//! End-to-end detection: regex + NER, merged, then redacted.
//!
//! The model half is live. The `pii-pipeline` this replaces hard-coded
//! `let model_entities = Vec::new();`, so only regex spans were ever detected and the
//! configured model was never consulted.

use crate::pii::config::PipelineConfig;
use crate::pii::context;
use crate::pii::engine::RegexEngine;
use crate::pii::merge::{merge_entities, MergedEntity};
use crate::pii::ner::{to_pii_category, NerDetector};
use crate::pii::types::{MergeConfig, MergePriority, ModelEntity};
use crate::pii::PiiError;
use crate::redaction::pseudonym::category_label;
use crate::redaction::{Pseudonymiser, RedactionAuditEntry, RedactionEngine, RedactionMode};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use web_time::Instant;

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
/// PipelineResult struct
pub struct PipelineResult {
    /// Input text with every merged span rewritten. Equal to the input for [`PiiPipeline::scan`].
    pub redacted_text: String,
    pub entities: Vec<MergedEntity>,
    pub audit_log: Vec<RedactionAuditEntry>,
    pub metrics: PipelineMetrics,
}

/// Per-stage timings and counts for one `process` call.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
/// PipelineMetrics struct
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
        let detector = build_detector(&config)?;
        Self::assemble(config, detector, None)
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
        Self::assemble(config, detector, None)
    }

    /// [`PiiPipeline::new`], plus the key set needed to mint reversible pseudonym tokens.
    ///
    /// Required when the configuration selects
    /// [`RedactionMode::Pseudonymize`](crate::redaction::RedactionMode::Pseudonymize);
    /// `new` and `with_detector` pass `None` and will fail for that mode rather than
    /// quietly masking instead.
    ///
    /// The `Arc` is shared rather than cloned per pipeline: every pipeline in a batch run
    /// must mint the same token for the same value, or cross-document co-reference — the
    /// point of the mode — breaks.
    ///
    /// # Errors
    ///
    /// As [`PiiPipeline::new`], plus
    /// [`RedactionError::MissingPseudonymKey`](crate::redaction::RedactionError::MissingPseudonymKey)
    /// if the mode needs a key and `pseudonymiser` is `None`.
    pub fn with_pseudonymiser(
        config: PipelineConfig,
        pseudonymiser: Option<Arc<Pseudonymiser>>,
    ) -> Result<Self, PiiError> {
        let detector = build_detector(&config)?;
        Self::assemble(config, detector, pseudonymiser)
    }

    /// [`PiiPipeline::with_detector`], plus the pseudonymisation key set.
    ///
    /// # Errors
    ///
    /// As [`PiiPipeline::with_pseudonymiser`], without the model-loading failures.
    pub fn with_detector_and_pseudonymiser(
        config: PipelineConfig,
        detector: Option<NerDetector>,
        pseudonymiser: Option<Arc<Pseudonymiser>>,
    ) -> Result<Self, PiiError> {
        Self::assemble(config, detector, pseudonymiser)
    }

    fn assemble(
        config: PipelineConfig,
        ner_detector: Option<NerDetector>,
        pseudonymiser: Option<Arc<Pseudonymiser>>,
    ) -> Result<Self, PiiError> {
        // Validated here, unconditionally, in every build profile — including
        // regex-only and wasm — rather than inside `load_detector`. `load_detector` is
        // only reached when `config.model.enabled` is true, so validating there would
        // silently accept a malformed vertical everywhere else. See `PiiError::InvalidVertical`.
        if let Some(vertical) = &config.vertical {
            vertical.validate()?;
        }

        if config.redaction.mode == RedactionMode::Pseudonymize {
            if let Some(detector) = &ner_detector {
                validate_ner_labels_for_pseudonymize(detector)?;
            }
        }

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
            redaction_engine: RedactionEngine::new(config.redaction.clone(), pseudonymiser)?,
            config,
        })
    }

/// config function
    pub fn config(&self) -> &PipelineConfig {
        &self.config
    }

    /// The audit-chain provenance value for this pipeline's configured vertical, if any.
    ///
    /// `None` when no `[pii.vertical]` is configured. See
    /// [`VerticalConfig::provenance_id`] and
    /// [`AuditEntry::vertical`](crate::audit::AuditEntry::vertical) for what the value
    /// means and why it is not the bare vertical id.
    pub(crate) fn vertical_provenance_id(&self) -> Option<String> {
        self.config
            .vertical
            .as_ref()
            .map(crate::pii::config::VerticalConfig::provenance_id)
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
        let redaction = self.redaction_engine.redact(text, &entities)?;
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
        let mut regex_entities = self.regex_engine.find_all(text);
        context::enhance(&mut regex_entities, text);
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

    /// Detect and redact using pre-computed model entities, bypassing the NER detector.
    ///
    /// This allows callers who have already run NER inference (e.g., for an entity glossary)
    /// to reuse those results instead of running inference a second time.
    ///
    /// # Errors
    ///
    /// Returns [`PiiError::Ner`] if regex detection fails, or
    /// [`PiiError::InvalidModelEntity`] if any pre-computed model entity fails
    /// validation (see [`validate_model_entity`]).
    pub async fn process_with_model_entities(
        &self,
        text: &str,
        model_entities: Vec<ModelEntity>,
    ) -> Result<PipelineResult, PiiError> {
        let start = Instant::now();
        let (entities, mut metrics) = self
            .detect_with_model_entities(text, model_entities)
            .await?;

        let redaction_start = Instant::now();
        let redaction = self.redaction_engine.redact(text, &entities)?;
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

    /// Detect without rewriting the text, using pre-computed model entities.
    ///
    /// See [`process_with_model_entities`] for details.
    pub async fn scan_with_model_entities(
        &self,
        text: &str,
        model_entities: Vec<ModelEntity>,
    ) -> Result<PipelineResult, PiiError> {
        let start = Instant::now();
        let (entities, mut metrics) = self
            .detect_with_model_entities(text, model_entities)
            .await?;
        metrics.total_ms = start.elapsed().as_millis() as u64;

        Ok(PipelineResult {
            redacted_text: text.to_string(),
            entities,
            audit_log: Vec::new(),
            metrics,
        })
    }

    async fn detect_with_model_entities(
        &self,
        text: &str,
        model_entities: Vec<ModelEntity>,
    ) -> Result<(Vec<MergedEntity>, PipelineMetrics), PiiError> {
        let mut metrics = PipelineMetrics::default();

        let regex_start = Instant::now();
        let mut regex_entities = self.regex_engine.find_all(text);
        context::enhance(&mut regex_entities, text);
        metrics.regex_ms = regex_start.elapsed().as_millis() as u64;

        // Use pre-computed model entities instead of running the detector. Validated,
        // not "trusted and not re-validated" as this used to say — see
        // `validate_model_entity`'s own doc comment for why a caller-supplied span can't
        // be handed to `RedactionEngine::redact` as-is.
        for entity in &model_entities {
            validate_model_entity(text, entity)?;
        }
        metrics.model_ms = 0; // No model inference performed

        let merge_start = Instant::now();
        let entities = merge_entities(regex_entities, model_entities, &self.merge_config);
        metrics.merge_ms = merge_start.elapsed().as_millis() as u64;
        metrics.entities_detected = entities.len() as u32;

        Ok((entities, metrics))
    }
}

/// Validates one caller-supplied [`ModelEntity`] before it's allowed anywhere near
/// [`merge_entities`]/[`RedactionEngine::redact`].
///
/// `process_with_model_entities`/`scan_with_model_entities` exist so a caller who already
/// ran NER inference (e.g. for an entity glossary) doesn't pay for it twice — but that
/// means these spans never pass through this pipeline's own detector, only whatever
/// produced them upstream (a WASM bridge, a different process, a different language
/// entirely). `RedactionEngine::redact` skips a reversed, out-of-bounds, or
/// non-UTF-8-boundary span rather than panicking on it, which is the right behavior for
/// *its* contract but the wrong one here: silently skipping means `redacted_text` comes
/// back with that PII span left completely unredacted, with nothing in the result to
/// indicate anything was wrong. Rejecting up front with a real error is the only way a
/// caller finds out.
fn validate_model_entity(text: &str, entity: &ModelEntity) -> Result<(), PiiError> {
    let invalid = |reason: String| PiiError::InvalidModelEntity {
        start: entity.start,
        end: entity.end,
        reason,
    };

    if entity.start >= entity.end {
        return Err(invalid("start must be less than end".to_string()));
    }
    let (start, end) = (entity.start as usize, entity.end as usize);
    if end > text.len() {
        return Err(invalid(format!(
            "end exceeds text length ({} bytes)",
            text.len()
        )));
    }
    if !text.is_char_boundary(start) || !text.is_char_boundary(end) {
        return Err(invalid(
            "span does not fall on a UTF-8 character boundary".to_string(),
        ));
    }
    if !entity.confidence.is_finite() || !(0.0..=1.0).contains(&entity.confidence) {
        return Err(invalid(format!(
            "confidence {} is not a finite value in [0, 1]",
            entity.confidence
        )));
    }
    if text[start..end] != entity.text {
        return Err(invalid(
            "text does not match the content of the span it claims to cover".to_string(),
        ));
    }
    Ok(())
}

#[cfg(all(feature = "ner-candle", not(target_arch = "wasm32")))]
fn load_detector(config: &PipelineConfig) -> Result<NerDetector, PiiError> {
    let model_dir = config.model.model_dir.as_deref().ok_or_else(|| {
        PiiError::ModelUnavailable("model.enabled is true but no model_dir is set".into())
    })?;
    let mut detector =
        NerDetector::from_candle_local(model_dir, config.model.lora_adapter_dir.as_deref())?
            .with_threshold(config.model_threshold_default);
    if let Some(vertical) = &config.vertical {
        // Extend, not replace: a finance vertical must still find people. See
        // `ner::categories_with_vertical`.
        detector =
            detector.with_categories(crate::pii::ner::categories_with_vertical(Some(vertical)));
    }
    Ok(detector)
}

#[cfg(not(all(feature = "ner-candle", not(target_arch = "wasm32"))))]
fn load_detector(_config: &PipelineConfig) -> Result<NerDetector, PiiError> {
    Err(PiiError::ModelUnavailable(
        "this build cannot load a model from disk; enable the `ner-candle` feature or supply a \
         detector via PiiPipeline::with_detector"
            .into(),
    ))
}

/// Build the detector `config` selects, or `None` if `config.model.enabled` is false.
///
/// `pub(crate)` so `HaciendaFacade` (P3a) can build the (expensive) detector once and
/// reuse it — via [`NerDetector`]'s cheap `Clone` — across one [`PiiPipeline`] per
/// tenant, instead of every pipeline independently reloading the model. `PiiPipeline::new`/
/// `with_pseudonymiser` also call this, so the "load only when enabled" branch exists in
/// exactly one place.
pub(crate) fn build_detector(config: &PipelineConfig) -> Result<Option<NerDetector>, PiiError> {
    if config.model.enabled {
        Ok(Some(load_detector(config)?))
    } else {
        Ok(None)
    }
}

/// Validate that every entity category configured on `detector` can be encoded in a
/// pseudonym token.
///
/// Only [`PiiCategory::Custom`] values can be problematic — the built-in categories are
/// all plain identifiers with no delimiter characters. This function is called at
/// construction time when the mode is `Pseudonymize`, so a bad label is rejected before
/// any document is processed.
///
/// # Errors
///
/// [`PiiError::InvalidEntityLabel`] naming the first label that fails.
fn validate_ner_labels_for_pseudonymize(detector: &NerDetector) -> Result<(), PiiError> {
    for category in detector.configured_categories() {
        let pii_category = to_pii_category(category);
        if let Err(e) = category_label(&pii_category) {
            let (label, reason) = match e {
                crate::redaction::PseudonymError::UnsupportedCategory { category, reason } => {
                    (category, reason)
                }
                other => (pii_category.to_string(), other.to_string()),
            };
            return Err(PiiError::InvalidEntityLabel { label, reason });
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pii::types::PiiCategory;
    use crate::redaction::RedactionMode;
    use crate::tenancy::TenantId;
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

    fn model_entity(category: PiiCategory, text: &str, start: u32, confidence: f32) -> ModelEntity {
        ModelEntity {
            category,
            text: text.to_string(),
            start,
            end: start + text.len() as u32,
            confidence,
        }
    }

    #[tokio::test]
    async fn should_accept_a_valid_model_entity() {
        let text = "Alice Martin works here";
        let entity = model_entity(PiiCategory::Person, "Alice Martin", 0, 0.95);
        let result = pipeline(vec![])
            .process_with_model_entities(text, vec![entity])
            .await
            .unwrap();

        assert_eq!(result.entities.len(), 1);
        assert!(!result.redacted_text.contains("Alice Martin"));
    }

    #[tokio::test]
    async fn should_reject_a_model_entity_with_a_reversed_span() {
        let text = "Alice Martin works here";
        let entity = ModelEntity {
            start: 12,
            end: 0,
            ..model_entity(PiiCategory::Person, "Alice Martin", 0, 0.95)
        };
        let err = pipeline(vec![])
            .process_with_model_entities(text, vec![entity])
            .await
            .unwrap_err();
        assert!(matches!(err, PiiError::InvalidModelEntity { .. }));
    }

    #[tokio::test]
    async fn should_reject_a_model_entity_out_of_bounds() {
        let text = "short";
        let entity = ModelEntity {
            end: 999,
            ..model_entity(PiiCategory::Person, "short", 0, 0.95)
        };
        let err = pipeline(vec![])
            .process_with_model_entities(text, vec![entity])
            .await
            .unwrap_err();
        assert!(matches!(err, PiiError::InvalidModelEntity { .. }));
    }

    #[tokio::test]
    async fn should_reject_a_model_entity_off_a_utf8_boundary() {
        // byte 1 is inside the multi-byte "é" — not a valid char boundary.
        let text = "émail";
        let entity = ModelEntity {
            start: 1,
            end: 2,
            ..model_entity(PiiCategory::Person, "m", 1, 0.95)
        };
        let err = pipeline(vec![])
            .process_with_model_entities(text, vec![entity])
            .await
            .unwrap_err();
        assert!(matches!(err, PiiError::InvalidModelEntity { .. }));
    }

    #[tokio::test]
    async fn should_reject_a_model_entity_with_non_finite_confidence() {
        let text = "Alice Martin works here";
        let entity = model_entity(PiiCategory::Person, "Alice Martin", 0, f32::NAN);
        let err = pipeline(vec![])
            .process_with_model_entities(text, vec![entity])
            .await
            .unwrap_err();
        assert!(matches!(err, PiiError::InvalidModelEntity { .. }));
    }

    #[tokio::test]
    async fn should_reject_a_model_entity_whose_text_does_not_match_its_span() {
        let text = "Alice Martin works here";
        // Span [0, 12) is "Alice Martin" in `text`, but the entity claims different text.
        let entity = ModelEntity {
            end: 12,
            ..model_entity(PiiCategory::Person, "Bob Jones", 0, 0.95)
        };
        let err = pipeline(vec![])
            .process_with_model_entities(text, vec![entity])
            .await
            .unwrap_err();
        assert!(matches!(err, PiiError::InvalidModelEntity { .. }));
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
    async fn should_boost_ssn_confidence_when_a_context_word_is_nearby() {
        let result = PiiPipeline::with_detector(config(), None)
            .unwrap()
            .scan("my ssn is 123-45-6789 on file")
            .await
            .unwrap();

        assert_eq!(result.entities.len(), 1);
        // Base confidence for a structurally-plausible-but-unverified SSN is 0.4
        // (`patterns::builtin_patterns`); "ssn" nearby should boost it.
        assert!(
            result.entities[0].confidence > 0.4,
            "expected a context boost, got {}",
            result.entities[0].confidence
        );
    }

    #[tokio::test]
    async fn should_not_flag_a_luhn_invalid_number_as_a_credit_card_end_to_end() {
        let result = PiiPipeline::with_detector(config(), None)
            .unwrap()
            .scan("invoice number 1234567890123456")
            .await
            .unwrap();

        assert!(result.entities.is_empty());
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
    fn should_refuse_to_build_when_a_custom_entity_category_label_contains_a_token_delimiter_and_mode_is_pseudonymize(
    ) {
        use crate::pii::types::PiiCategory;
        use crate::redaction::pseudonym::{EnvKeyResolver, ACTIVE_KEY_VAR, KEY_BYTES};

        let pseudonymiser = {
            let resolver = EnvKeyResolver::with_lookup(|name| match name {
                ACTIVE_KEY_VAR => Some("k1".to_string()),
                "HACIENDA_PSEUDONYM_KEY_K1" => Some("07".repeat(KEY_BYTES)),
                _ => None,
            });
            Some(Arc::new(
                crate::redaction::Pseudonymiser::new(&resolver, &TenantId::default_tenant(), &[])
                    .unwrap(),
            ))
        };

        let bad_category = PiiCategory::Custom("has:colon".into());
        let detector = NerDetector::new(Arc::new(StubBackend(vec![]))).with_categories(vec![
            xberg::types::entity::EntityCategory::Custom("has:colon".into()),
        ]);

        let pipeline_config = PipelineConfig {
            redaction: crate::redaction::RedactionConfig {
                mode: RedactionMode::Pseudonymize,
                ..Default::default()
            },
            ..Default::default()
        };

        let result = PiiPipeline::with_detector_and_pseudonymiser(
            pipeline_config,
            Some(detector),
            pseudonymiser,
        );
        assert!(
            result.is_err(),
            "pipeline construction must fail for a label containing ':'"
        );
        let msg = result.err().unwrap().to_string();
        assert!(
            msg.contains("has:colon") || msg.contains("colon"),
            "error should name the offending label: {msg}"
        );
        // Also verify that a valid custom label succeeds with Pseudonymize
        let _ = bad_category;
    }

    #[test]
    fn should_accept_a_custom_entity_category_with_a_valid_label_in_pseudonymize_mode() {
        use crate::redaction::pseudonym::{EnvKeyResolver, ACTIVE_KEY_VAR, KEY_BYTES};

        let pseudonymiser = {
            let resolver = EnvKeyResolver::with_lookup(|name| match name {
                ACTIVE_KEY_VAR => Some("k1".to_string()),
                "HACIENDA_PSEUDONYM_KEY_K1" => Some("07".repeat(KEY_BYTES)),
                _ => None,
            });
            Some(Arc::new(
                crate::redaction::Pseudonymiser::new(&resolver, &TenantId::default_tenant(), &[])
                    .unwrap(),
            ))
        };

        let detector = NerDetector::new(Arc::new(StubBackend(vec![]))).with_categories(vec![
            xberg::types::entity::EntityCategory::Custom("employee_id".into()),
        ]);

        let pipeline_config = PipelineConfig {
            redaction: crate::redaction::RedactionConfig {
                mode: RedactionMode::Pseudonymize,
                ..Default::default()
            },
            ..Default::default()
        };

        assert!(
            PiiPipeline::with_detector_and_pseudonymiser(
                pipeline_config,
                Some(detector),
                pseudonymiser
            )
            .is_ok(),
            "a label with no delimiters must be accepted"
        );
    }

    // ── Task 2: Tier 0 schema verticals ───────────────────────────────────────

    fn bad_vertical() -> crate::pii::config::VerticalConfig {
        crate::pii::config::VerticalConfig {
            id: "finance".into(),
            labels: vec!["has:colon".into()],
        }
    }

    fn valid_vertical() -> crate::pii::config::VerticalConfig {
        // Deliberately not a label that any built-in regex pattern (SWIFT/BIC, IBAN,
        // SSN, etc. — see `patterns.rs`) would also claim, so this test isolates the
        // vertical/model path rather than exercising regex-first merge priority.
        crate::pii::config::VerticalConfig {
            id: "finance".into(),
            labels: vec!["docket_number".into()],
        }
    }

    fn env_pseudonymiser() -> Arc<crate::redaction::Pseudonymiser> {
        use crate::redaction::pseudonym::{EnvKeyResolver, ACTIVE_KEY_VAR, KEY_BYTES};
        let resolver = EnvKeyResolver::with_lookup(|name| match name {
            ACTIVE_KEY_VAR => Some("k1".to_string()),
            "HACIENDA_PSEUDONYM_KEY_K1" => Some("07".repeat(KEY_BYTES)),
            _ => None,
        });
        Arc::new(
            crate::redaction::Pseudonymiser::new(&resolver, &TenantId::default_tenant(), &[])
                .unwrap(),
        )
    }

    #[test]
    fn should_reject_a_vertical_label_containing_a_token_delimiter() {
        // Validation is not pseudonymise-only: it must fail in every redaction mode.
        for mode in [RedactionMode::Mask, RedactionMode::Hash] {
            let config = PipelineConfig {
                redaction: crate::redaction::RedactionConfig {
                    mode,
                    ..Default::default()
                },
                vertical: Some(bad_vertical()),
                ..Default::default()
            };
            let result = PiiPipeline::with_detector(config, None);
            assert!(
                matches!(result, Err(PiiError::InvalidVertical { .. })),
                "mode {mode:?} must still reject a delimiter-containing label"
            );
        }

        // Also fails under Pseudonymize, with no detector configured at all.
        let config = PipelineConfig {
            redaction: crate::redaction::RedactionConfig {
                mode: RedactionMode::Pseudonymize,
                ..Default::default()
            },
            vertical: Some(bad_vertical()),
            ..Default::default()
        };
        assert!(matches!(
            PiiPipeline::with_detector(config, None),
            Err(PiiError::InvalidVertical { .. })
        ));
    }

    #[test]
    fn should_reject_an_invalid_vertical_in_a_regex_only_pipeline() {
        // model.enabled = false: load_detector is never called. This pins the
        // validation site to `assemble` — a future refactor that moves validation back
        // into `load_detector` would make this test pass with `Ok(_)` instead.
        let config = PipelineConfig {
            model: crate::pii::config::ModelConfig {
                enabled: false,
                ..Default::default()
            },
            vertical: Some(bad_vertical()),
            ..Default::default()
        };
        assert!(matches!(
            PiiPipeline::new(config),
            Err(PiiError::InvalidVertical { .. })
        ));
    }

    #[tokio::test]
    async fn should_pseudonymise_a_vertical_detection_into_a_parsable_token() {
        let pseudonymiser = env_pseudonymiser();
        let vertical = valid_vertical();

        let text = "DOC-REF-48213";
        let detector = NerDetector::new(Arc::new(StubBackend(vec![Entity {
            category: EntityCategory::Custom("docket_number".into()),
            text: text.into(),
            start: 0,
            end: text.len() as u32,
            confidence: Some(0.9),
        }])))
        .with_categories(crate::pii::ner::categories_with_vertical(Some(&vertical)));

        let pipeline_config = PipelineConfig {
            redaction: crate::redaction::RedactionConfig {
                mode: RedactionMode::Pseudonymize,
                ..Default::default()
            },
            vertical: Some(vertical),
            ..Default::default()
        };

        let pipeline = PiiPipeline::with_detector_and_pseudonymiser(
            pipeline_config,
            Some(detector),
            Some(Arc::clone(&pseudonymiser)),
        )
        .unwrap();

        let result = pipeline.process(text).await.unwrap();
        let start = result.redacted_text.find('[').expect(&result.redacted_text);
        let end = result.redacted_text.find(']').expect(&result.redacted_text);
        let token = &result.redacted_text[start..=end];

        assert!(
            token.starts_with("[DOCKET_NUMBER:"),
            "token must be labelled with the uppercased vertical label, got {token}"
        );
        // `reveal` returns the normalised value (lowercased, NFKC) rather than the
        // original casing — that lossiness is `normalize`'s documented behaviour, not
        // something specific to verticals.
        assert_eq!(pseudonymiser.reveal(token).unwrap(), text.to_lowercase());
    }

    #[test]
    fn should_reject_a_pipeline_whose_vertical_label_cannot_be_pseudonymised() {
        // `config.vertical` is intentionally left `None` here — this test targets the
        // *pre-existing* `validate_ner_labels_for_pseudonymize`, called with a detector
        // whose categories were built the same way `load_detector` builds them for a
        // vertical (`categories_with_vertical`), to confirm it already covers
        // vertical-origin labels with no new code of its own.
        let vertical = bad_vertical();
        let categories = crate::pii::ner::categories_with_vertical(Some(&vertical));
        let detector = NerDetector::new(Arc::new(StubBackend(vec![]))).with_categories(categories);

        let pseudonymiser = env_pseudonymiser();
        let pipeline_config = PipelineConfig {
            redaction: crate::redaction::RedactionConfig {
                mode: RedactionMode::Pseudonymize,
                ..Default::default()
            },
            ..Default::default()
        };

        let result = PiiPipeline::with_detector_and_pseudonymiser(
            pipeline_config,
            Some(detector),
            Some(pseudonymiser),
        );
        assert!(matches!(result, Err(PiiError::InvalidEntityLabel { .. })));
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
