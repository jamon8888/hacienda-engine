//! Applies a [`RedactionMode`] to detected spans and emits a hash-chained audit log.

use super::types::{
    RedactionAuditEntry, RedactionConfig, RedactionMetrics, RedactionMode, RedactionResult,
};
use crate::pii::merge::MergedEntity;

/// Genesis value for the per-result redaction hash chain.
const GENESIS_CHAIN_HASH: &str = "0000000000000000000000000000000000000000000000000000000000000000";

pub struct RedactionEngine {
    mode: RedactionMode,
    custom_template: Option<String>,
}

impl RedactionEngine {
    pub fn new(config: RedactionConfig) -> Self {
        Self {
            mode: config.mode,
            custom_template: config.custom_template,
        }
    }

    /// Rewrite every span in `entities` according to the configured mode.
    ///
    /// `entities` must be offset-ordered and non-overlapping — the output of
    /// [`merge_entities`](crate::pii::merge::merge_entities) satisfies both. Spans that
    /// would go backwards or past the end of `text` are skipped rather than panicking,
    /// so a caller passing unsorted spans loses redactions but never crashes.
    pub fn redact(&self, text: &str, entities: &[MergedEntity]) -> RedactionResult {
        let start = std::time::Instant::now();

        let mut output = String::with_capacity(text.len());
        let mut last_end = 0usize;
        let mut audit_log = Vec::with_capacity(entities.len());
        let mut chain_hash = GENESIS_CHAIN_HASH.to_string();
        let mut redacted = 0u32;

        for entity in entities {
            let (span_start, span_end) = (entity.start as usize, entity.end as usize);
            if span_start < last_end || span_end > text.len() || span_start >= span_end {
                continue;
            }
            if !text.is_char_boundary(span_start) || !text.is_char_boundary(span_end) {
                continue;
            }

            output.push_str(&text[last_end..span_start]);

            let original = &text[span_start..span_end];
            output.push_str(&self.replacement_for(entity, original));

            let span_hash = blake3::hash(original.as_bytes()).to_hex().to_string();
            chain_hash = chain_next(&chain_hash, &entity.category.to_string(), &span_hash);

            audit_log.push(RedactionAuditEntry {
                category: entity.category.to_string(),
                action: self.mode,
                source: entity.source,
                span_hash,
                span_length: entity.end - entity.start,
                confidence: Some(entity.confidence),
                timestamp: unix_timestamp(),
                chain_hash: chain_hash.clone(),
            });

            redacted += 1;
            last_end = span_end;
        }
        output.push_str(&text[last_end..]);

        RedactionResult {
            text: output,
            audit_log,
            metrics: RedactionMetrics {
                redaction_ms: start.elapsed().as_millis() as u64,
                entities_detected: entities.len() as u32,
                entities_redacted: redacted,
            },
        }
    }

    fn replacement_for(&self, entity: &MergedEntity, original: &str) -> String {
        match self.mode {
            RedactionMode::Mask => entity.redact_template.clone(),
            RedactionMode::Hash => {
                let hash = blake3::hash(original.as_bytes());
                hash.as_bytes()[..4]
                    .iter()
                    .map(|b| format!("{b:02x}"))
                    .collect()
            }
            RedactionMode::Pseudonymize => format!("[{:?}:****]", entity.category).to_uppercase(),
            RedactionMode::Remove => String::new(),
            RedactionMode::Custom => self
                .custom_template
                .as_ref()
                .map(|t| {
                    t.replace("{{ENTITY}}", &entity.category.to_string())
                        .replace("{{TEXT}}", original)
                })
                .unwrap_or_else(|| format!("[{:?}]", entity.category).to_uppercase()),
        }
    }
}

fn chain_next(previous: &str, category: &str, span_hash: &str) -> String {
    let mut hasher = blake3::Hasher::new();
    hasher.update(previous.as_bytes());
    hasher.update(category.as_bytes());
    hasher.update(span_hash.as_bytes());
    hasher.finalize().to_hex().to_string()
}

fn unix_timestamp() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pii::types::{EntitySource, PiiCategory};

    fn entity(category: PiiCategory, start: u32, end: u32) -> MergedEntity {
        MergedEntity {
            redact_template: format!("[{category:?}]").to_uppercase(),
            category,
            text: String::new(),
            start,
            end,
            confidence: 1.0,
            source: EntitySource::Regex,
            format_preserving: false,
        }
    }

    fn engine(mode: RedactionMode) -> RedactionEngine {
        RedactionEngine::new(RedactionConfig {
            mode,
            ..Default::default()
        })
    }

    #[test]
    fn should_mask_a_span_with_its_template() {
        let text = "mail alice@x.io ok";
        let result = engine(RedactionMode::Mask).redact(text, &[entity(PiiCategory::Email, 5, 15)]);
        assert_eq!(result.text, "mail [EMAIL] ok");
    }

    #[test]
    fn should_delete_a_span_in_remove_mode() {
        let text = "mail alice@x.io ok";
        let result =
            engine(RedactionMode::Remove).redact(text, &[entity(PiiCategory::Email, 5, 15)]);
        assert_eq!(result.text, "mail  ok");
    }

    #[test]
    fn should_pseudonymize_a_span_with_a_category_placeholder() {
        let text = "mail alice@x.io ok";
        let result =
            engine(RedactionMode::Pseudonymize).redact(text, &[entity(PiiCategory::Email, 5, 15)]);
        assert_eq!(result.text, "mail [EMAIL:****] ok");
    }

    #[test]
    fn should_expand_placeholders_in_custom_mode() {
        let text = "mail alice@x.io ok";
        let engine = RedactionEngine::new(RedactionConfig {
            mode: RedactionMode::Custom,
            custom_template: Some("<{{ENTITY}}>".into()),
            preserve_format: true,
        });
        assert_eq!(
            engine
                .redact(text, &[entity(PiiCategory::Email, 5, 15)])
                .text,
            "mail <Email> ok"
        );
    }

    #[test]
    fn should_return_the_input_unchanged_when_there_are_no_entities() {
        let text = "nothing to redact";
        assert_eq!(engine(RedactionMode::Mask).redact(text, &[]).text, text);
    }

    #[test]
    fn should_record_the_span_digest_not_the_span_itself() {
        let text = "mail alice@x.io ok";
        let result = engine(RedactionMode::Mask).redact(text, &[entity(PiiCategory::Email, 5, 15)]);
        assert_eq!(result.audit_log.len(), 1);
        let entry = &result.audit_log[0];
        assert_eq!(
            entry.span_hash,
            blake3::hash(b"alice@x.io").to_hex().to_string()
        );
        assert_eq!(entry.span_length, 10);
        assert_ne!(entry.chain_hash, GENESIS_CHAIN_HASH);
    }

    #[test]
    fn should_chain_audit_hashes_across_spans() {
        let text = "a@b.io and c@d.io";
        let result = engine(RedactionMode::Mask).redact(
            text,
            &[
                entity(PiiCategory::Email, 0, 6),
                entity(PiiCategory::Email, 11, 17),
            ],
        );
        assert_eq!(result.audit_log.len(), 2);
        assert_ne!(
            result.audit_log[0].chain_hash,
            result.audit_log[1].chain_hash
        );
    }

    #[test]
    fn should_skip_a_span_that_goes_past_the_end_of_the_text() {
        let text = "short";
        let result = engine(RedactionMode::Mask).redact(text, &[entity(PiiCategory::Email, 2, 99)]);
        assert_eq!(result.text, text);
        assert_eq!(result.metrics.entities_redacted, 0);
        assert_eq!(result.metrics.entities_detected, 1);
    }

    #[test]
    fn should_skip_a_span_that_splits_a_multibyte_character() {
        // 'é' occupies bytes 1..3, so byte 2 lands inside it.
        let text = "héllo";
        let result = engine(RedactionMode::Mask).redact(text, &[entity(PiiCategory::Email, 2, 4)]);
        assert_eq!(result.text, text);
        assert_eq!(result.metrics.entities_redacted, 0);
    }

    #[test]
    fn should_redact_a_span_after_a_multibyte_character() {
        let text = "héllo a@b.io";
        let start = text.find("a@b.io").unwrap() as u32;
        let result = engine(RedactionMode::Mask)
            .redact(text, &[entity(PiiCategory::Email, start, start + 6)]);
        assert_eq!(result.text, "héllo [EMAIL]");
    }
}
