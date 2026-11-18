//! The bridge from email operations to hacienda-core's real PII engine.
//!
//! The original plugin called `hacienda_core::pii::{PiiDetector, PiiRule}` —
//! APIs that never existed. This module builds one [`HaciendaFacade`] with PII
//! enabled and routes every email body through it, so email redaction gets the
//! same detection pipeline, audit chain and pseudonymisation as the rest of
//! hacienda. It is shared (behind `&self`) by all operations in `EmailProcessor`.

use crate::config::{PrivacyMode, RedactionRule};
use crate::error::{EmailError, Result};
use hacienda_core::facade::TextRedactResult;
use hacienda_core::pii::PipelineConfig;
use hacienda_core::{HaciendaConfig, HaciendaFacade, SpanText};
use std::collections::HashMap;
use std::sync::Arc;

/// Shared, thread-safe handle to the real hacienda PII engine.
#[derive(Clone)]
pub struct PiiEngine {
    facade: Arc<HaciendaFacade>,
}

impl PiiEngine {
    /// Build the engine for a tenant's chosen privacy mode (design D1).
    pub fn new(mode: PrivacyMode) -> Result<Self> {
        let facade = match mode {
            PrivacyMode::Redact => {
                let config = HaciendaConfig::default().with_pii(PipelineConfig::default());
                HaciendaFacade::new(config)
            }
            PrivacyMode::Pseudonymize => {
                // Reversible tokens need a key resolver. EnvKeyResolver reads the
                // key material from the environment (the same convention hacienda-core
                // uses); a tenant without keys configured gets a startup error rather
                // than a silent downgrade to masking.
                use hacienda_core::redaction::pseudonym::EnvKeyResolver;
                use hacienda_core::redaction::{RedactionConfig as CoreRedactionConfig, RedactionMode};
                let mut pipeline = PipelineConfig::default();
                pipeline.redaction = CoreRedactionConfig {
                    mode: RedactionMode::Pseudonymize,
                    ..CoreRedactionConfig::default()
                };
                let config = HaciendaConfig::default().with_pii(pipeline);
                HaciendaFacade::with_key_resolver(config, Arc::new(EnvKeyResolver::new()), &[])
            }
        }
        .map_err(|e| EmailError::Redaction(format!("failed to build PII engine: {e}")))?;
        Ok(Self { facade: Arc::new(facade) })
    }

    /// Redact all configured PII categories in `text`, returning the rewritten
    /// text and per-category counts.
    ///
    /// `rules`, `replacement` and `preserve_format` are accepted for API
    /// compatibility with the plugin config. The authoritative detection and
    /// redaction behaviour is hacienda-core's pipeline; plugin-level rule
    /// filtering is applied to the counts so callers still see the categories
    /// they asked for. The configured replacement string is honoured via the
    /// custom redaction template when it differs from the category default.
    pub async fn redact(
        &self,
        text: &str,
        rules: &[RedactionRule],
        replacement: &str,
        preserve_format: bool,
    ) -> Result<(String, HashMap<String, usize>)> {
        let allowed: Vec<String> = rules.iter().map(category_name).collect();

        // Scan for entity categories first (omit plaintext — we never need it).
        let scan = self
            .facade
            .scan_text(text, SpanText::Omit)
            .await
            .map_err(|e| EmailError::Redaction(format!("PII scan failed: {e}")))?;

        let mut stats: HashMap<String, usize> = HashMap::new();
        for entity in &scan.entities {
            let name = entity.category.to_string();
            if allowed.is_empty() || allowed.iter().any(|a| category_matches(a, &name)) {
                *stats.entry(name).or_insert(0) += 1;
            }
        }

        // Redaction via the real facade. mode/preserve_format come from the
        // pipeline config; a caller-specific replacement is applied by rewriting
        // the category templates in a dedicated pipeline when requested.
        let result = if replacement == "[REDACTED]" && preserve_format {
            self.facade
                .redact_text(text)
                .await
                .map_err(|e| EmailError::Redaction(format!("redaction failed: {e}")))?
        } else {
            self.redact_with_template(text, replacement, preserve_format).await?
        };

        Ok((result.redacted_text, stats))
    }

    /// Build a one-off facade whose pipeline uses the caller's custom template
    /// and preserve-format flag. Built per call but only when the caller deviates
    /// from defaults, so the common path reuses the shared engine.
    async fn redact_with_template(
        &self,
        text: &str,
        template: &str,
        preserve_format: bool,
    ) -> Result<TextRedactResult> {
        use hacienda_core::redaction::{RedactionConfig as CoreRedactionConfig, RedactionMode};

        let mut pipeline = PipelineConfig::default();
        pipeline.redaction = CoreRedactionConfig {
            mode: RedactionMode::Custom,
            custom_template: Some(template.to_string()),
            preserve_format,
            key_id: None,
        };
        let config = HaciendaConfig::default().with_pii(pipeline);
        let facade = HaciendaFacade::new(config)
            .map_err(|e| EmailError::Redaction(format!("failed to build custom PII engine: {e}")))?;
        facade
            .redact_text(text)
            .await
            .map_err(|e| EmailError::Redaction(format!("custom redaction failed: {e}")))
    }

    /// Scan text for PII entities, returning (label, mention text, confidence,
    /// span) tuples. `SpanText::Omit` clears the entity text in the result, so
    /// the mention is recovered from the original text via the reported offsets.
    pub async fn scan_entities(
        &self,
        text: &str,
    ) -> Result<Vec<(String, String, f32, (usize, usize))>> {
        let scan = self
            .facade
            .scan_text(text, SpanText::Omit)
            .await
            .map_err(|e| EmailError::Ner(format!("PII scan failed: {e}")))?;
        Ok(scan
            .entities
            .into_iter()
            .map(|e| {
                let start = e.start as usize;
                let end = (e.end as usize).min(text.len());
                let mention = if start <= end {
                    text.get(start..end).unwrap_or_default().to_string()
                } else {
                    String::new()
                };
                (
                    e.category.to_string(),
                    mention,
                    e.confidence,
                    (start, end),
                )
            })
            .collect())
    }
}

/// Map a plugin `RedactionRule` to the hacienda category name it corresponds to.
fn category_name(rule: &RedactionRule) -> String {
    match rule {
        RedactionRule::Email => "Email",
        RedactionRule::Phone => "PhoneNumber",
        RedactionRule::Ssn => "Ssn",
        RedactionRule::CreditCard => "CreditCard",
        RedactionRule::Iban => "Iban",
        RedactionRule::Ip => "IpAddress",
        RedactionRule::Custom { label, .. } => label,
    }
    .to_string()
}

/// Match a plugin rule name against a hacienda category display name, tolerating
/// the small naming differences (Phone vs PhoneNumber, Ip vs IpAddress).
fn category_matches(rule: &str, category: &str) -> bool {
    if rule.eq_ignore_ascii_case(category) {
        return true;
    }
    matches!(
        (rule.to_ascii_lowercase().as_str(), category.to_ascii_lowercase().as_str()),
        ("phone", "phonenumber")
            | ("ip", "ipaddress")
            | ("ssn", "usssn")
            | ("credit_card", "creditcard")
    )
}
