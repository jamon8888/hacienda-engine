//! Top-level configuration for [`crate::HaciendaFacade`].
//!
//! Every stage after extraction is optional and `None` means "do not run it". The
//! stage configuration types are the real ones from their own modules — there is no
//! parallel configuration taxonomy to keep in sync.

use crate::compliance::ComplianceConfig;
use crate::glossary::GlossaryConfig;
use crate::pii::PipelineConfig;
use crate::review::ReviewConfig;
use serde::{Deserialize, Serialize};
use xberg::ExtractionConfig;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct HaciendaConfig {
    pub extraction: ExtractionConfig,
    /// Detection and redaction. `None` extracts without touching PII.
    pub pii: Option<PipelineConfig>,
    pub compliance: Option<ComplianceConfig>,
    /// Human review queue for low-confidence detections.
    ///
    /// Requires `pii`: there is nothing to review without detections.
    pub review: Option<ReviewConfig>,
    pub glossary: Option<GlossaryConfig>,
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
