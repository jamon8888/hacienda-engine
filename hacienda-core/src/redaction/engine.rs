//! Applies a [`RedactionMode`] to detected spans and emits a hash-chained audit log.

use super::pseudonym::Pseudonymiser;
use super::types::{
    RedactionAuditEntry, RedactionConfig, RedactionMetrics, RedactionMode, RedactionResult,
};
use super::RedactionError;
use crate::audit::RedactionAction;
use crate::pii::merge::MergedEntity;
use std::sync::Arc;
use web_time::{Instant, SystemTime, UNIX_EPOCH};

/// Genesis value for the per-result redaction hash chain.
const GENESIS_CHAIN_HASH: &str = "0000000000000000000000000000000000000000000000000000000000000000";

#[derive(Debug)]
pub struct RedactionEngine {
    mode: RedactionMode,
    custom_template: Option<String>,
    /// Present only for [`RedactionMode::Pseudonymize`], and then always present —
    /// [`RedactionEngine::new`] refuses to build the pair the other way round.
    ///
    /// `Arc` because one key set is shared by every pipeline in the process: a batch run
    /// must not mint a different token per worker for the same value.
    pseudonymiser: Option<Arc<Pseudonymiser>>,
}

impl RedactionEngine {
    /// Build an engine for `config`.
    ///
    /// `pseudonymiser` is required for [`RedactionMode::Pseudonymize`] and ignored
    /// otherwise. Supplying one for another mode is not an error — a caller that always
    /// passes the process-wide key set should not have to branch on the mode.
    ///
    /// # Errors
    ///
    /// [`RedactionError::MissingPseudonymKey`] when the mode is `Pseudonymize` and no
    /// pseudonymiser is given. This fails rather than falling back to masking; see that
    /// variant's documentation.
    pub fn new(
        config: RedactionConfig,
        pseudonymiser: Option<Arc<Pseudonymiser>>,
    ) -> Result<Self, RedactionError> {
        if config.mode == RedactionMode::Pseudonymize && pseudonymiser.is_none() {
            return Err(RedactionError::MissingPseudonymKey);
        }
        Ok(Self {
            mode: config.mode,
            custom_template: config.custom_template,
            pseudonymiser,
        })
    }

    /// Rewrite every span in `entities` according to the configured mode.
    ///
    /// `entities` must be offset-ordered and non-overlapping — the output of
    /// [`merge_entities`](crate::pii::merge::merge_entities) satisfies both. Spans that
    /// would go backwards or past the end of `text` are skipped rather than panicking,
    /// so a caller passing unsorted spans loses redactions but never crashes.
    ///
    /// # Errors
    ///
    /// Returns [`RedactionError::Pseudonym`] if a span cannot be tokenised — in practice
    /// only a custom category whose name contains a token delimiter. The whole call fails
    /// rather than emitting a partially-redacted document: the alternative is returning
    /// text that looks redacted and still contains the span.
    pub fn redact(
        &self,
        text: &str,
        entities: &[MergedEntity],
    ) -> Result<RedactionResult, RedactionError> {
        let start = Instant::now();

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
            output.push_str(&self.replacement_for(entity, original)?);

            let span_hash = blake3::hash(original.as_bytes()).to_hex().to_string();
            chain_hash = chain_next(&chain_hash, &entity.category.to_string(), &span_hash);

            audit_log.push(RedactionAuditEntry {
                category: entity.category.to_string(),
                action: self.audit_action(),
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

        Ok(RedactionResult {
            text: output,
            audit_log,
            metrics: RedactionMetrics {
                redaction_ms: start.elapsed().as_millis() as u64,
                entities_detected: entities.len() as u32,
                entities_redacted: redacted,
            },
        })
    }

    fn replacement_for(
        &self,
        entity: &MergedEntity,
        original: &str,
    ) -> Result<String, RedactionError> {
        Ok(match self.mode {
            RedactionMode::Mask => entity.redact_template.clone(),
            RedactionMode::Hash => {
                let hash = blake3::hash(original.as_bytes());
                hash.as_bytes()[..4]
                    .iter()
                    .map(|b| format!("{b:02x}"))
                    .collect()
            }
            RedactionMode::Pseudonymize => self
                .pseudonymiser
                .as_ref()
                // Unreachable: `new` rejects this pairing. Erroring rather than
                // `expect`ing keeps a future refactor that loosens `new` from turning a
                // configuration mistake into a panic mid-document.
                .ok_or(RedactionError::MissingPseudonymKey)?
                .token(&entity.category, original)?,
            RedactionMode::Remove => String::new(),
            RedactionMode::Custom => self
                .custom_template
                .as_ref()
                .map(|t| {
                    t.replace("{{ENTITY}}", &entity.category.to_string())
                        .replace("{{TEXT}}", original)
                })
                .unwrap_or_else(|| format!("[{:?}]", entity.category).to_uppercase()),
        })
    }
}

impl RedactionEngine {
    /// The audit record of what this engine does to a span.
    ///
    /// This lives here rather than in a `From<RedactionMode>` impl because `Custom`
    /// carries the template that was applied, and the engine is the only place that value
    /// is in scope. The `None` arm mirrors `replacement_for`'s own fallback for `Custom`
    /// with no template configured, so the log describes the rewrite that actually
    /// happened rather than the one that was asked for.
    fn audit_action(&self) -> RedactionAction {
        match self.mode {
            RedactionMode::Mask => RedactionAction::Mask,
            RedactionMode::Hash => RedactionAction::Hash,
            RedactionMode::Pseudonymize => RedactionAction::Pseudonymize,
            RedactionMode::Remove => RedactionAction::Remove,
            RedactionMode::Custom => RedactionAction::Custom(
                self.custom_template
                    .clone()
                    .unwrap_or_else(|| "[{CATEGORY}] (no template configured)".to_string()),
            ),
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
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pii::types::{EntitySource, PiiCategory};
    use crate::redaction::pseudonym::{EnvKeyResolver, ACTIVE_KEY_VAR, KEY_BYTES};

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
        RedactionEngine::new(
            RedactionConfig {
                mode,
                ..Default::default()
            },
            pseudonymiser(),
        )
        .unwrap()
    }

    fn pseudonymiser() -> Option<Arc<Pseudonymiser>> {
        let resolver = EnvKeyResolver::with_lookup(|name| match name {
            ACTIVE_KEY_VAR => Some("k1".to_string()),
            "HACIENDA_PSEUDONYM_KEY_K1" => Some("07".repeat(KEY_BYTES)),
            _ => None,
        });
        let ctx = crate::tenancy::TenantCtx::default_tenant(crate::tenancy::ActorId::new("test"));
        Some(Arc::new(Pseudonymiser::new(&ctx, &resolver, &[]).unwrap()))
    }

    /// Shorthand for the common case: redaction that is expected to succeed.
    fn apply(engine: &RedactionEngine, text: &str, entities: &[MergedEntity]) -> RedactionResult {
        engine.redact(text, entities).unwrap()
    }

    #[test]
    fn should_mask_a_span_with_its_template() {
        let text = "mail alice@x.io ok";
        let result = apply(
            &engine(RedactionMode::Mask),
            text,
            &[entity(PiiCategory::Email, 5, 15)],
        );
        assert_eq!(result.text, "mail [EMAIL] ok");
    }

    #[test]
    fn should_delete_a_span_in_remove_mode() {
        let text = "mail alice@x.io ok";
        let result = apply(
            &engine(RedactionMode::Remove),
            text,
            &[entity(PiiCategory::Email, 5, 15)],
        );
        assert_eq!(result.text, "mail  ok");
    }

    #[test]
    fn should_pseudonymize_a_span_with_a_reversible_token() {
        // Replaces an assertion of the constant "mail [EMAIL:****] ok". The shape and the
        // round trip are asserted rather than a literal ciphertext, which would only
        // re-encode whatever the implementation happens to produce.
        let text = "mail alice@x.io ok";
        let engine = engine(RedactionMode::Pseudonymize);
        let out = apply(&engine, text, &[entity(PiiCategory::Email, 5, 15)]).text;

        assert!(out.starts_with("mail [EMAIL:k1:"), "{out}");
        assert!(out.ends_with("] ok"), "{out}");
        assert!(!out.contains("alice@x.io"), "the span survived: {out}");

        let token = &out["mail ".len()..out.len() - " ok".len()];
        assert_eq!(
            engine
                .pseudonymiser
                .as_ref()
                .unwrap()
                .reveal(token)
                .unwrap(),
            "alice@x.io"
        );
    }

    #[test]
    fn should_give_two_occurrences_of_one_value_the_same_token_in_one_document() {
        // Co-reference is the entire point of the mode: a reader must be able to tell
        // that both mentions are the same person without learning who.
        let text = "a@b.io wrote, then a@b.io replied";
        let out = apply(
            &engine(RedactionMode::Pseudonymize),
            text,
            &[
                entity(PiiCategory::Email, 0, 6),
                entity(PiiCategory::Email, 19, 25),
            ],
        )
        .text;
        assert_eq!(out.matches("[EMAIL:").count(), 2, "{out}");
        let first = &out[out.find('[').unwrap()..=out.find(']').unwrap()];
        assert_eq!(out.matches(first).count(), 2, "{out}");
    }

    #[test]
    fn should_give_two_different_values_different_tokens() {
        let text = "a@b.io and c@d.io";
        let out = apply(
            &engine(RedactionMode::Pseudonymize),
            text,
            &[
                entity(PiiCategory::Email, 0, 6),
                entity(PiiCategory::Email, 11, 17),
            ],
        )
        .text;
        let first = &out[out.find('[').unwrap()..=out.find(']').unwrap()];
        assert_eq!(out.matches(first).count(), 1, "{out}");
    }

    #[test]
    fn should_refuse_to_build_a_pseudonymize_engine_without_a_key() {
        // Must not silently degrade to masking: that applies a weaker, irreversible
        // control than the one the operator asked for.
        let config = RedactionConfig {
            mode: RedactionMode::Pseudonymize,
            ..Default::default()
        };
        assert!(matches!(
            RedactionEngine::new(config, None),
            Err(RedactionError::MissingPseudonymKey)
        ));
    }

    #[test]
    fn should_build_the_other_modes_without_a_key() {
        for mode in [
            RedactionMode::Mask,
            RedactionMode::Hash,
            RedactionMode::Remove,
            RedactionMode::Custom,
        ] {
            let config = RedactionConfig {
                mode,
                ..Default::default()
            };
            assert!(
                RedactionEngine::new(config, None).is_ok(),
                "{mode:?} needs no secret"
            );
        }
    }

    #[test]
    fn should_fail_the_whole_document_rather_than_emit_an_unredacted_span() {
        // A custom category carrying a token delimiter cannot be tokenised. Returning
        // text that looks redacted but still holds the span would be the worse outcome.
        let engine = engine(RedactionMode::Pseudonymize);
        let text = "mail alice@x.io ok";
        let bad = entity(PiiCategory::Custom("has:colon".into()), 5, 15);
        assert!(engine.redact(text, &[bad]).is_err());
    }

    #[test]
    fn should_not_expose_key_material_in_debug_output() {
        let rendered = format!("{:?}", engine(RedactionMode::Pseudonymize));
        assert!(!rendered.contains('7'), "{rendered}");
    }

    #[test]
    fn should_expand_placeholders_in_custom_mode() {
        let text = "mail alice@x.io ok";
        let engine = RedactionEngine::new(
            RedactionConfig {
                mode: RedactionMode::Custom,
                custom_template: Some("<{{ENTITY}}>".into()),
                ..Default::default()
            },
            None,
        )
        .unwrap();
        assert_eq!(
            apply(&engine, text, &[entity(PiiCategory::Email, 5, 15)]).text,
            "mail <Email> ok"
        );
    }

    #[test]
    fn should_record_which_custom_template_was_applied_in_the_audit_log() {
        // The audit log's whole purpose is to let an auditor reconstruct what rewrite was
        // performed. Recording a constant for every Custom redaction makes each one
        // indistinguishable from the rest, which is the same as recording nothing.
        let engine = RedactionEngine::new(
            RedactionConfig {
                mode: RedactionMode::Custom,
                custom_template: Some("<{{ENTITY}}>".into()),
                ..Default::default()
            },
            None,
        )
        .unwrap();
        let result = apply(
            &engine,
            "mail alice@x.io ok",
            &[entity(PiiCategory::Email, 5, 15)],
        );
        assert_eq!(
            result.audit_log[0].action,
            RedactionAction::Custom("<{{ENTITY}}>".into())
        );
    }

    #[test]
    fn should_record_the_applied_action_for_every_mode() {
        for (mode, expected) in [
            (RedactionMode::Mask, RedactionAction::Mask),
            (RedactionMode::Hash, RedactionAction::Hash),
            (RedactionMode::Remove, RedactionAction::Remove),
        ] {
            let result = apply(
                &engine(mode),
                "mail alice@x.io ok",
                &[entity(PiiCategory::Email, 5, 15)],
            );
            assert_eq!(result.audit_log[0].action, expected, "mode {mode:?}");
        }
    }

    #[test]
    fn should_record_a_custom_redaction_with_no_template_as_the_fallback_it_applied() {
        // `replacement_for` falls back to `[CATEGORY]` when Custom is configured without a
        // template. The audit entry must describe that, not claim a template was used.
        let engine = RedactionEngine::new(
            RedactionConfig {
                mode: RedactionMode::Custom,
                custom_template: None,
                ..Default::default()
            },
            None,
        )
        .unwrap();
        let result = apply(
            &engine,
            "mail alice@x.io ok",
            &[entity(PiiCategory::Email, 5, 15)],
        );
        let RedactionAction::Custom(recorded) = &result.audit_log[0].action else {
            panic!(
                "expected a Custom action, got {:?}",
                result.audit_log[0].action
            );
        };
        assert!(
            recorded.contains("no template configured"),
            "the audit entry does not say a template was absent: {recorded}"
        );
    }

    #[test]
    fn should_return_the_input_unchanged_when_there_are_no_entities() {
        let text = "nothing to redact";
        assert_eq!(apply(&engine(RedactionMode::Mask), text, &[]).text, text);
    }

    #[test]
    fn should_record_the_span_digest_not_the_span_itself() {
        let text = "mail alice@x.io ok";
        let result = apply(
            &engine(RedactionMode::Mask),
            text,
            &[entity(PiiCategory::Email, 5, 15)],
        );
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
        let result = apply(
            &engine(RedactionMode::Mask),
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
        let result = apply(
            &engine(RedactionMode::Mask),
            text,
            &[entity(PiiCategory::Email, 2, 99)],
        );
        assert_eq!(result.text, text);
        assert_eq!(result.metrics.entities_redacted, 0);
        assert_eq!(result.metrics.entities_detected, 1);
    }

    #[test]
    fn should_skip_a_span_that_splits_a_multibyte_character() {
        // 'é' occupies bytes 1..3, so byte 2 lands inside it.
        let text = "héllo";
        let result = apply(
            &engine(RedactionMode::Mask),
            text,
            &[entity(PiiCategory::Email, 2, 4)],
        );
        assert_eq!(result.text, text);
        assert_eq!(result.metrics.entities_redacted, 0);
    }

    #[test]
    fn should_redact_a_span_after_a_multibyte_character() {
        let text = "héllo a@b.io";
        let start = text.find("a@b.io").unwrap() as u32;
        let result = apply(
            &engine(RedactionMode::Mask),
            text,
            &[entity(PiiCategory::Email, start, start + 6)],
        );
        assert_eq!(result.text, "héllo [EMAIL]");
    }
}
