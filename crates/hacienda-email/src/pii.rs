//! Bridge from email operations to the host's tenant-scoped PII authority.
//!
//! P0.4: the `HaciendaFacade` is never built here. The HTTP API injects the very
//! facade held by `ApiState`, so pseudonym keys, tenant scoping and the audit
//! chain are the host's — not an ephemeral facade with `Caller::Trusted` and an
//! `EnvKeyResolver` read from the process environment. Every operation takes the
//! request's [`Caller`]; there is deliberately no convenience method that could
//! silently fall back to the default tenant.

use crate::config::RedactionRule;
use crate::error::{EmailError, Result};
use hacienda_core::auth::Caller;
use hacienda_core::facade::TextRedactResult;
use hacienda_core::pii::{PiiCategory, PipelineConfig};
use hacienda_core::{HaciendaConfig, HaciendaFacade, SpanText};
use std::collections::HashMap;
use std::sync::Arc;

/// A handle to the host PII authority, shared by all email operations.
///
/// Constructed from the facade the host already owns. `Clone` is cheap (an
/// `Arc` bump) and lets the async HTTP job carry the same authority into a
/// `tokio::spawn`ed task without rebuilding it (P0.4/P0.5).
#[derive(Clone)]
pub struct PiiEngine {
    facade: Arc<HaciendaFacade>,
}

impl PiiEngine {
    /// Wrap the host's facade. The HTTP API passes `ApiState::facade.clone()`.
    ///
    /// There is intentionally no `new(mode)` here: selecting a privacy mode at
    /// an email call site would rebuild the facade and detach it from the
    /// authenticated tenant's key resolver and shared audit store.
    pub fn from_facade(facade: Arc<HaciendaFacade>) -> Self {
        Self { facade }
    }

    /// The shared facade. Exposed so callers can read the shared audit tip and
    /// reveal metadata tokens through the exact same authority that minted
    /// them — never through a locally constructed pseudonymiser.
    pub fn facade(&self) -> &Arc<HaciendaFacade> {
        &self.facade
    }

    /// Redact all configured PII categories in `text` as `caller`'s tenant.
    ///
    /// `rules` are applied to the returned per-category counts. `replacement`
    /// and `preserve_format` are accepted for plugin-config compatibility; the
    /// authoritative redaction template lives in the host facade's pipeline,
    /// which is the only place a tenant's pseudonym key and audit store are
    /// configured. A caller-specific template would require a new pipeline
    /// (hence a new facade) and is therefore not honoured here.
    pub async fn redact(
        &self,
        caller: Caller<'_>,
        text: &str,
        rules: &[RedactionRule],
        _replacement: &str,
        _preserve_format: bool,
    ) -> Result<(String, HashMap<String, usize>)> {
        let allowed: Vec<String> = rules.iter().map(category_name).collect();

        // Scan with plaintext omitted: the mention text is never needed and
        // `SpanText::Include` would additionally require `PiiReveal`.
        let scan = self
            .facade
            .scan_text_with_auth(caller, text, SpanText::Omit)
            .await
            .map_err(|e| EmailError::Redaction(format!("PII scan failed: {e}")))?;

        let mut stats: HashMap<String, usize> = HashMap::new();
        for entity in &scan.entities {
            let name = entity.category.to_string();
            if allowed.is_empty() || allowed.iter().any(|a| category_matches(a, &name)) {
                *stats.entry(name).or_insert(0) += 1;
            }
        }

        let result: TextRedactResult = self
            .facade
            .redact_text_with_auth(caller, text)
            .await
            .map_err(|e| EmailError::Redaction(format!("redaction failed: {e}")))?;

        Ok((result.redacted_text, stats))
    }

    /// Scan text for PII entities as `caller`'s tenant, returning
    /// `(label, mention, confidence, span)` tuples.
    pub async fn scan_entities(
        &self,
        caller: Caller<'_>,
        text: &str,
    ) -> Result<Vec<(String, String, f32, (usize, usize))>> {
        let scan = self
            .facade
            .scan_text_with_auth(caller, text, SpanText::Omit)
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
                (e.category.to_string(), mention, e.confidence, (start, end))
            })
            .collect())
    }

    /// Project one metadata value to an opaque, tenant-keyed token through the
    /// host facade's pseudonymiser (the same authority reveal uses).
    ///
    /// This replaces the old local FNV-1a hash: no new secret is generated, no
    /// unkeyed hash is invented, and the token is only reversible with the
    /// caller's tenant key via `HaciendaFacade::reveal_token_with_auth`. The
    /// `domain` is folded into the category label so a subject token can never
    /// collide with an address token for the same value.
    pub async fn project_metadata(
        &self,
        caller: Caller<'_>,
        domain: &str,
        value: &str,
    ) -> Result<String> {
        let category = PiiCategory::Custom(format!("email.{domain}"));
        self.facade
            .mint_pseudonym_token_with_auth(caller, &category, value)
            .await
            .map_err(|e| {
                EmailError::Redaction(format!(
                    "tenant-keyed metadata projection failed for domain `{domain}`: {e}"
                ))
            })
    }

    /// Build a facade for local/in-process callers (CLI, MCP, unit tests).
    ///
    /// Not used by the HTTP email path: there the host facade is always
    /// injected via [`Self::from_facade`]. The resolver is supplied by the host,
    /// never discovered here, so no local secret or environment fallback exists.
    pub fn for_local_use(
        config: HaciendaConfig,
        resolver: Arc<dyn hacienda_core::redaction::KeyResolver>,
    ) -> Result<Self> {
        let facade = HaciendaFacade::with_key_resolver(config, resolver, &[])
            .map_err(|e| EmailError::Redaction(format!("failed to build local PII facade: {e}")))?;
        Ok(Self {
            facade: Arc::new(facade),
        })
    }

    /// Default local pipeline config used by in-process callers and tests.
    pub fn local_config() -> HaciendaConfig {
        HaciendaConfig::default().with_pii(PipelineConfig::default())
    }

    /// Deterministic resolver for single-tenant unit tests only. Never used by
    /// the HTTP path, and never a fallback: an explicit resolver is mandatory.
    #[doc(hidden)]
    pub fn for_local_testing() -> Self {
        use hacienda_core::redaction::pseudonym::{
            EnvKeyResolver, ACTIVE_KEY_VAR, KEY_BYTES, KEY_VAR_PREFIX,
        };
        let resolver = Arc::new(EnvKeyResolver::with_lookup(move |name| match name {
            ACTIVE_KEY_VAR => Some("k1".to_string()),
            n if n == format!("{KEY_VAR_PREFIX}K1") => Some("07".repeat(KEY_BYTES)),
            _ => None,
        }));
        Self::for_local_use(Self::local_config(), resolver)
            .expect("local test facade with a key resolver")
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
        (
            rule.to_ascii_lowercase().as_str(),
            category.to_ascii_lowercase().as_str()
        ),
        ("phone", "phonenumber")
            | ("ip", "ipaddress")
            | ("ssn", "usssn")
            | ("credit_card", "creditcard")
    )
}
