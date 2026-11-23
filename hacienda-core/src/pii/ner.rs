//! Statistical PII detection, delegated to an xberg NER backend.
//!
//! hacienda does not implement inference. It consumes [`xberg::text::ner::NerBackend`],
//! which is what the `pii-fastino` stub this replaced never did — that stub returned an
//! empty vector, so the model half of the pipeline detected nothing.

use crate::pii::types::{ModelEntity, PiiCategory};
use crate::pii::PiiError;
use std::sync::Arc;
use xberg::text::ner::NerBackend;
use xberg::types::entity::{Entity, EntityCategory};

/// Categories requested from the backend when a caller does not narrow the set.
///
/// `pub(crate)` rather than private: `pipeline::load_detector`'s `ner-candle` arm needs
/// to *extend* this set with a configured vertical's labels rather than replace it —
/// see [`categories_with_vertical`].
pub(crate) const DEFAULT_CATEGORIES: &[EntityCategory] = &[
    EntityCategory::Person,
    EntityCategory::Organization,
    EntityCategory::Location,
    EntityCategory::Email,
    EntityCategory::Phone,
];

/// [`DEFAULT_CATEGORIES`], extended with `vertical`'s labels as `EntityCategory::Custom`
/// entries when one is configured.
///
/// The base five are always kept: a vertical only adds detectable labels, it never
/// narrows what the base pipeline finds — a finance vertical must still find people.
/// This is a free function rather than inline in `pipeline::load_detector` so the
/// "extend, don't replace" behaviour can be unit-tested directly, without a real model
/// directory: `load_detector`'s `ner-candle` arm is gated on `not(target_arch =
/// "wasm32")` and needs real weights on disk to run at all, which this workspace's test
/// sandbox does not have.
///
/// `cfg`-gated to match its only callers (`load_detector`'s `ner-candle` arm, and
/// tests) — without this, a default-feature (`ner-candle` off) non-test build of
/// `hacienda-core` sees no caller at all and emits a `dead_code` warning.
#[cfg(any(test, all(feature = "ner-candle", not(target_arch = "wasm32"))))]
pub(crate) fn categories_with_vertical(
    vertical: Option<&crate::pii::config::VerticalConfig>,
) -> Vec<EntityCategory> {
    let mut categories = DEFAULT_CATEGORIES.to_vec();
    if let Some(vertical) = vertical {
        categories.extend(vertical.labels.iter().cloned().map(EntityCategory::Custom));
    }
    categories
}

/// Runs an xberg NER backend and maps its entities into pipeline spans.
///
/// `Clone` is cheap (P3a): `backend` is already behind an `Arc`, and `categories`/
/// `threshold` are small owned data. This is what lets [`HaciendaFacade`]
/// (`hacienda-core/src/facade.rs`) build one detector once and construct a second,
/// per-tenant [`crate::pii::PiiPipeline`] from a clone of it without reloading the NER
/// backend's model weights.
///
/// [`HaciendaFacade`]: crate::HaciendaFacade
#[derive(Clone)]
pub struct NerDetector {
    backend: Arc<dyn NerBackend>,
    categories: Vec<EntityCategory>,
    threshold: f32,
}

impl NerDetector {
    /// Wrap any xberg NER backend.
    pub fn new(backend: Arc<dyn NerBackend>) -> Self {
        Self {
            backend,
            categories: DEFAULT_CATEGORIES.to_vec(),
            threshold: 0.0,
        }
    }

    /// Restrict detection to `categories`. An empty slice means "whatever the backend finds".
    pub fn with_categories(mut self, categories: Vec<EntityCategory>) -> Self {
        self.categories = categories;
        self
    }

    /// Drop entities the backend scores below `threshold`.
    ///
    /// Entities without a confidence score are kept: a backend that does not report
    /// confidence should not be silently filtered to nothing.
    ///
    /// # Candle GLiNER threshold floor
    ///
    /// `xberg::text::ner::candle::CandleBackend::detect` hard-codes an internal
    /// threshold of `0.5` before `NerDetector`'s own filter runs. Consequently
    /// `with_threshold` can only ever *raise* the effective threshold above `0.5`;
    /// it can never lower it. Entities the Candle backend scores below `0.5` are
    /// invisible at any configured threshold, so every recall figure measured
    /// through this detector is *truncated recall at 0.5*, not recall. This is an
    /// upstream xberg limitation, not something this crate can work around without
    /// patching `CandleBackend::detect`.
    pub fn with_threshold(mut self, threshold: f32) -> Self {
        self.threshold = threshold;
        self
    }

    /// The entity categories this detector is configured to request from the backend.
    pub(crate) fn configured_categories(&self) -> &[EntityCategory] {
        &self.categories
    }

    /// Load the Candle GLiNER2 backend from a local model directory, optionally
    /// merging a LoRA adapter.
    ///
    /// Backends are cached process-wide by `(model_dir, lora_adapter_dir)`, so the
    /// model-load and adapter-merge cost is paid once per pair rather than per document.
    ///
    /// If `expected_weights_hash` is `Some`, the blake3 digest of
    /// `model_dir/model.safetensors` is computed and compared before loading; a
    /// mismatch aborts with [`PiiError::Ner`] so a tampered or corrupted model can
    /// never be executed.
    ///
    /// # Errors
    ///
    /// Returns [`PiiError::Ner`] if the model or adapter cannot be loaded, or if
    /// `expected_weights_hash` is set and the weights digest does not match.
    #[cfg(all(feature = "ner-candle", not(target_arch = "wasm32")))]
    pub fn from_candle_local(
        model_dir: &std::path::Path,
        lora_adapter_dir: Option<&std::path::Path>,
        expected_weights_hash: Option<&str>,
    ) -> Result<Self, PiiError> {
        if let Some(expected) = expected_weights_hash {
            let weights_path = model_dir.join("model.safetensors");
            let weights = std::fs::read(&weights_path).map_err(|source| PiiError::Ner {
                message: format!(
                    "reading model weights from {}",
                    weights_path.display()
                ),
                source: Box::new(xberg::XbergError::Io(source)),
            })?;
            let digest = blake3::hash(&weights).to_hex().to_string();
            if digest != expected {
                return Err(PiiError::Ner {
                    message: format!(
                        "model weights integrity check failed: expected {}, got {}",
                        expected, digest
                    ),
                    source: Box::new(xberg::XbergError::Io(std::io::Error::other(
                        "weights hash mismatch",
                    ))),
                });
            }
        }
        let backend =
            xberg::text::ner::candle::CandleBackend::get_or_init(model_dir, lora_adapter_dir)
                .map_err(|source| PiiError::Ner {
                    message: format!("loading Candle GLiNER2 from {}", model_dir.display()),
                    source: Box::new(source),
                })?;
        Ok(Self::new(backend))
    }

    /// [`from_candle_local`](Self::from_candle_local)'s wasm32 sibling: loads the same
    /// Candle GLiNER2 backend from in-memory bytes rather than a filesystem model
    /// directory — a browser has no disk to read `model_dir` from. The caller (Studio)
    /// already fetches and IndexedDB-caches these three buffers for its own NER pass;
    /// this does not fetch anything itself.
    ///
    /// # Errors
    ///
    /// Returns [`PiiError::Ner`] if the model bytes cannot be loaded, or if
    /// `expected_weights_hash` is set and the weights digest does not match.
    #[cfg(all(target_arch = "wasm32", feature = "ner-candle-wasm"))]
    pub fn from_candle_bytes(
        weights: &[u8],
        tokenizer_json: &[u8],
        encoder_config_json: &[u8],
        expected_weights_hash: Option<&str>,
    ) -> Result<Self, PiiError> {
        if let Some(expected) = expected_weights_hash {
            let digest = blake3::hash(weights).to_hex().to_string();
            if digest != expected {
                return Err(PiiError::Ner {
                    message: format!(
                        "model weights integrity check failed: expected {}, got {}",
                        expected, digest
                    ),
                    source: Box::new(xberg::XbergError::Io(std::io::Error::other(
                        "weights hash mismatch",
                    ))),
                });
            }
        }
        let backend = xberg::text::ner::candle::CandleBackend::from_bytes(
            weights,
            tokenizer_json,
            encoder_config_json,
        )
        .map_err(|source| PiiError::Ner {
            message: "loading Candle GLiNER2 from in-memory bytes".into(),
            source: Box::new(source),
        })?;
        Ok(Self::new(Arc::new(backend)))
    }

    /// Detect entities in `text`.
    ///
    /// # Errors
    ///
    /// Returns [`PiiError::Ner`] if the backend fails.
    pub async fn detect(&self, text: &str) -> Result<Vec<ModelEntity>, PiiError> {
        if text.is_empty() {
            return Ok(Vec::new());
        }

        let entities = self
            .backend
            .detect(text, &self.categories)
            .await
            .map_err(|source| PiiError::Ner {
                message: "NER backend inference failed".into(),
                source: Box::new(source),
            })?;

        Ok(entities
            .into_iter()
            .filter(|e| e.confidence.is_none_or(|c| c >= self.threshold))
            .map(to_model_entity)
            .collect())
    }
}

fn to_model_entity(entity: Entity) -> ModelEntity {
    ModelEntity {
        category: to_pii_category(&entity.category),
        // A backend that does not report confidence is treated as certain; the merge
        // step then falls back to regex-first priority rather than dropping the span.
        confidence: entity.confidence.unwrap_or(1.0),
        text: entity.text,
        start: entity.start,
        end: entity.end,
    }
}

/// Map an xberg entity category onto the PII taxonomy.
///
/// Categories with no PII counterpart (Money, Percent, Date, Time) are carried through
/// as `Custom` rather than dropped, so a caller redacting everything still sees them.
pub fn to_pii_category(category: &EntityCategory) -> PiiCategory {
    match category {
        EntityCategory::Person => PiiCategory::Person,
        EntityCategory::Organization => PiiCategory::Organization,
        EntityCategory::Location => PiiCategory::Address,
        EntityCategory::Email => PiiCategory::Email,
        EntityCategory::Phone => PiiCategory::PhoneNumber,
        EntityCategory::Url => PiiCategory::Url,
        EntityCategory::Date => PiiCategory::Custom("Date".into()),
        EntityCategory::Time => PiiCategory::Custom("Time".into()),
        EntityCategory::Money => PiiCategory::Custom("Money".into()),
        EntityCategory::Percent => PiiCategory::Custom("Percent".into()),
        EntityCategory::Custom(label) => match label.to_ascii_lowercase().as_str() {
            "ssn" | "social_security_number" => PiiCategory::Ssn,
            "credit_card" | "creditcard" => PiiCategory::CreditCard,
            "iban" => PiiCategory::Iban,
            "passport" | "passport_number" => PiiCategory::PassportNumber,
            "address" => PiiCategory::Address,
            "full_name" | "fullname" | "name" => PiiCategory::FullName,
            "person" => PiiCategory::Person,
            "first_name" => PiiCategory::FirstName,
            "middle_name" => PiiCategory::MiddleName,
            "last_name" => PiiCategory::LastName,
            "date_of_birth" => PiiCategory::DateOfBirth,
            "email" => PiiCategory::Email,
            "phone_number" | "phone" => PiiCategory::PhoneNumber,
            "street_address" => PiiCategory::StreetAddress,
            "city" => PiiCategory::City,
            "state_or_region" => PiiCategory::StateOrRegion,
            "postal_code" => PiiCategory::PostalCode,
            "country" => PiiCategory::Country,
            "government_id" => PiiCategory::GovernmentId,
            "national_id_number" => PiiCategory::NationalId,
            "drivers_license_number" | "license_number" => PiiCategory::DriversLicense,
            "tax_id" | "tax_number" => PiiCategory::TaxId,
            "bank_account" | "account_number" => PiiCategory::BankAccount,
            "routing_number" => PiiCategory::RoutingNumber,
            "payment_card" => PiiCategory::PaymentCard,
            "card_number" => PiiCategory::CreditCard,
            "card_expiry" => PiiCategory::CardExpiry,
            "card_cvv" => PiiCategory::CardCvv,
            "username" => PiiCategory::Username,
            "ip_address" => PiiCategory::IpAddress,
            "password" => PiiCategory::Password,
            "secret" => PiiCategory::SecretToken,
            "api_key" => PiiCategory::ApiKey,
            "access_token" | "recovery_code" => PiiCategory::SecretToken,
            "url" => PiiCategory::Url,
            _ => PiiCategory::Custom(label.clone()),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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

    fn entity(category: EntityCategory, text: &str, start: u32, confidence: Option<f32>) -> Entity {
        Entity {
            category,
            text: text.into(),
            start,
            end: start + text.len() as u32,
            confidence,
        }
    }

    fn detector(entities: Vec<Entity>) -> NerDetector {
        NerDetector::new(Arc::new(StubBackend(entities)))
    }

    #[tokio::test]
    async fn should_return_nothing_for_empty_text() {
        let detector = detector(vec![entity(EntityCategory::Person, "Alice", 0, Some(0.9))]);
        assert!(detector.detect("").await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn should_map_a_person_entity_onto_the_person_category() {
        let detected = detector(vec![entity(EntityCategory::Person, "Alice", 0, Some(0.9))])
            .detect("Alice works here")
            .await
            .unwrap();
        assert_eq!(detected.len(), 1);
        assert_eq!(detected[0].category, PiiCategory::Person);
        assert_eq!(detected[0].text, "Alice");
        assert_eq!(detected[0].confidence, 0.9);
    }

    #[tokio::test]
    async fn should_map_location_onto_address() {
        let detected = detector(vec![entity(
            EntityCategory::Location,
            "Paris",
            0,
            Some(0.8),
        )])
        .detect("Paris")
        .await
        .unwrap();
        assert_eq!(detected[0].category, PiiCategory::Address);
    }

    #[tokio::test]
    async fn should_treat_a_missing_confidence_as_certain() {
        let detected = detector(vec![entity(EntityCategory::Person, "Bob", 0, None)])
            .detect("Bob")
            .await
            .unwrap();
        assert_eq!(detected[0].confidence, 1.0);
    }

    #[tokio::test]
    async fn should_drop_entities_below_the_threshold() {
        let detected = detector(vec![
            entity(EntityCategory::Person, "Alice", 0, Some(0.2)),
            entity(EntityCategory::Person, "Bob", 10, Some(0.95)),
        ])
        .with_threshold(0.5)
        .detect("Alice and Bob")
        .await
        .unwrap();
        assert_eq!(detected.len(), 1);
        assert_eq!(detected[0].text, "Bob");
    }

    #[test]
    fn should_recognise_known_custom_labels_as_pii_categories() {
        assert_eq!(
            to_pii_category(&EntityCategory::Custom("SSN".into())),
            PiiCategory::Ssn
        );
        assert_eq!(
            to_pii_category(&EntityCategory::Custom("credit_card".into())),
            PiiCategory::CreditCard
        );
    }

    #[test]
    fn should_carry_an_unknown_custom_label_through_unchanged() {
        assert_eq!(
            to_pii_category(&EntityCategory::Custom("EmployeeId".into())),
            PiiCategory::Custom("EmployeeId".into())
        );
    }

    #[test]
    fn should_extend_base_categories_with_vertical_labels() {
        let vertical = crate::pii::config::VerticalConfig {
            id: "finance".into(),
            labels: vec!["swift_code".into(), "account_number".into()],
        };
        let categories = categories_with_vertical(Some(&vertical));

        for base in DEFAULT_CATEGORIES {
            assert!(
                categories.contains(base),
                "extending must keep base category {base:?}"
            );
        }
        for label in &vertical.labels {
            assert!(
                categories.contains(&EntityCategory::Custom(label.clone())),
                "missing vertical label {label}"
            );
        }
        assert_eq!(
            categories.len(),
            DEFAULT_CATEGORIES.len() + vertical.labels.len(),
            "vertical labels must be additive, not a replacement"
        );
    }

    #[test]
    fn should_return_the_base_categories_unchanged_when_no_vertical_is_configured() {
        assert_eq!(categories_with_vertical(None), DEFAULT_CATEGORIES.to_vec());
    }

    #[tokio::test]
    async fn should_map_a_vertical_label_to_a_custom_pii_category() {
        let detected = detector(vec![entity(
            EntityCategory::Custom("swift_code".into()),
            "BOFAUS3N",
            0,
            Some(0.9),
        )])
        .with_categories(vec![EntityCategory::Custom("swift_code".into())])
        .detect("BOFAUS3N")
        .await
        .unwrap();

        assert_eq!(detected.len(), 1);
        assert_eq!(
            detected[0].category,
            PiiCategory::Custom("swift_code".into())
        );
    }

    #[tokio::test]
    async fn should_map_an_aliased_vertical_label_to_its_taxonomy_category() {
        // A vertical author who names a label "iban" collides with `to_pii_category`'s
        // alias table and gets `PiiCategory::Iban`, not `Custom("iban")`. Documented as
        // a footgun on `VerticalConfig`.
        let detected = detector(vec![entity(
            EntityCategory::Custom("iban".into()),
            "FR7630006000011234567890189",
            0,
            Some(0.9),
        )])
        .with_categories(vec![EntityCategory::Custom("iban".into())])
        .detect("FR7630006000011234567890189")
        .await
        .unwrap();

        assert_eq!(detected.len(), 1);
        assert_eq!(detected[0].category, PiiCategory::Iban);
        assert_ne!(detected[0].category, PiiCategory::Custom("iban".into()));
    }
}
