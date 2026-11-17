//! Top-level configuration for [`crate::HaciendaFacade`].
//!
//! Every stage after extraction is optional and `None` means "do not run it". The
//! stage configuration types are the real ones from their own modules — there is no
//! parallel configuration taxonomy to keep in sync.

use crate::auth::authn::AuthConfig;
use crate::compliance::ComplianceConfig;
use crate::glossary::GlossaryConfig;
use crate::pii::PipelineConfig;
use crate::review::ReviewConfig;
use hacienda_installation::InstallationConfig;
use serde::{Deserialize, Serialize};
use xberg::ExtractionConfig;

/// # Unknown keys are rejected
///
/// This type and every hacienda-owned config below it carry `deny_unknown_fields`, so a
/// misspelt key fails the load instead of being discarded. Silently discarding produces
/// a file that says a control is configured and a process in which it is not — the
/// failure shape the CLI spec (§6.3) names as the most common "why is this not
/// redacting" incident.
///
/// The usual objection is forward compatibility: a file written for a newer hacienda
/// stops loading on an older one. That trade is already made for us — xberg's
/// [`ExtractionConfig`] declares `deny_unknown_fields`, so `[extraction]` is strict
/// today. Leaving hacienda's own sections permissive would make the section we do not
/// own the stricter one.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct HaciendaConfig {
    pub extraction: ExtractionConfig,
    /// Which installation this process belongs to: `LocalFull` or `TeamOnPremise`.
    ///
    /// This is the first-class `[installation]` section of issue #142. It is the *same*
    /// type the desktop client reads from `HACIENDA_INSTALLATION_CONFIG`, resolved by
    /// [`hacienda_installation::resolve_installation`] and validated by
    /// [`InstallationConfig::validate_for`], so client and server cannot grow two ideas
    /// of what an installation means (dossier `docs/wayfinder/131-installations.md` §I1,
    /// gap É1).
    ///
    /// `None` means no source declared one. That is correct for the one-shot commands —
    /// `hacienda extract` runs inside its own process boundary and needs no topology —
    /// but `hacienda serve` refuses to start without it: a network-facing process has to
    /// say which installation it is rather than be assumed to be one.
    pub installation: Option<InstallationConfig>,
    /// Detection and redaction. `None` extracts without touching PII.
    pub pii: Option<PipelineConfig>,
    pub compliance: Option<ComplianceConfig>,
    /// Human review queue for low-confidence detections.
    ///
    /// Requires `pii`: there is nothing to review without detections.
    pub review: Option<ReviewConfig>,
    pub glossary: Option<GlossaryConfig>,
    /// Authentication for `hacienda serve`. Disabled by default, which is correct for
    /// the CLI and the desktop app: there, the process boundary *is* the trust boundary.
    ///
    /// It is not a field the CLI reads — `hacienda extract` is always `Caller::Trusted`.
    /// It lives here so that one config file describes the whole product, and so that
    /// `hacienda serve` can refuse a non-loopback bind when it is off.
    pub auth: AuthConfig,
}

impl HaciendaConfig {
    /// Enable PII detection and redaction with default settings.
    pub fn with_pii(mut self, config: PipelineConfig) -> Self {
        self.pii = Some(config);
        self
    }

    /// The audit configuration, which lives inside the PII pipeline configuration
    /// because there is nothing to audit without detections.
    pub fn audit(&self) -> Option<&crate::audit::AuditConfig> {
        self.pii.as_ref().map(|p| &p.audit)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn should_run_extraction_only_by_default() {
        let config = HaciendaConfig::default();
        assert!(config.pii.is_none());
        assert!(config.compliance.is_none());
        assert!(config.review.is_none());
        assert!(config.glossary.is_none());
        assert!(config.audit().is_none());
    }

    #[test]
    fn should_expose_the_audit_config_once_pii_is_enabled() {
        let config = HaciendaConfig::default().with_pii(PipelineConfig::default());
        assert!(config.audit().is_some());
    }
}
