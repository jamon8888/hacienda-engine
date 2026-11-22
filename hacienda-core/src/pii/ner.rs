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

/// Runtime identity of the NER backend a detector actually holds.
///
/// This lives on the detector, not in a process-global cell: two surfaces in one
/// process can load two different verified models, and a single global would
/// silently report whichever loaded first. It also separates "a model is loaded"
/// from "that model has a pinned registry id" — an unpinned/private model from
/// `from_candle_local` is still model-backed even though it has no id to report.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NerRuntimeState {
    /// A real model backend is loaded (native Candle or wasm Candle), whether or
    /// not it carries a pinned registry id.
    pub model_loaded: bool,
    /// Pinned id from a verified load; `None` for unpinned/private models and for
    /// regex-only surfaces that never built a detector.
    pub model_id: Option<Arc<str>>,
}

impl NerRuntimeState {
    /// No model backend in use: regex-only, or a reporting surface that never
    /// constructed a facade (e.g. `hacienda capabilities`).
    pub fn unavailable() -> Self {
        Self {
            model_loaded: false,
            model_id: None,
        }
    }

    #[cfg(any(test, feature = "ner-candle", feature = "ner-candle-wasm"))]
    pub(crate) fn loaded(model_id: Option<Arc<str>>) -> Self {
        Self {
            model_loaded: true,
            model_id,
        }
    }
}

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
    /// Identity of the backend this detector holds. Reporting surfaces read it
    /// from the detector they actually use instead of a process-global guess.
    runtime: NerRuntimeState,
}

impl NerDetector {
    /// Wrap any xberg NER backend without claiming a loaded model.
    ///
    /// Model-loading constructors below override this via [`Self::with_runtime`].
    /// Callers supplying their own backend (e.g. test stubs) are regex-only from
    /// the capability report's point of view until they state otherwise.
    pub fn new(backend: Arc<dyn NerBackend>) -> Self {
        Self::with_runtime(backend, NerRuntimeState::unavailable())
    }

    fn with_runtime(backend: Arc<dyn NerBackend>, runtime: NerRuntimeState) -> Self {
        Self {
            backend,
            categories: DEFAULT_CATEGORIES.to_vec(),
            threshold: 0.0,
            runtime,
        }
    }

    /// Runtime identity of the backend this detector actually holds.
    pub fn runtime_state(&self) -> NerRuntimeState {
        self.runtime.clone()
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
    /// The resulting detector reports `runtime_ready = true` with no model id: a
    /// privately fine-tuned/unpinned model is still model-backed even though there
    /// is no registry identity to report.
    ///
    /// # Errors
    ///
    /// Returns [`PiiError::Ner`] if the model or adapter cannot be loaded.
    #[cfg(all(feature = "ner-candle", not(target_arch = "wasm32")))]
    pub fn from_candle_local(
        model_dir: &std::path::Path,
        lora_adapter_dir: Option<&std::path::Path>,
    ) -> Result<Self, PiiError> {
        let backend =
            xberg::text::ner::candle::CandleBackend::get_or_init(model_dir, lora_adapter_dir)
                .map_err(|source| PiiError::Ner {
                    message: format!("loading Candle GLiNER2 from {}", model_dir.display()),
                    source: Box::new(source),
                })?;
        Ok(Self::with_runtime(backend, NerRuntimeState::loaded(None)))
    }

    /// Load a pinned GLiNER2 model, verifying the local directory against the
    /// shared registry before any bytes reach Candle.
    ///
    /// The host still owns download and caching; this guard only enforces the
    /// integrity contract at the seam. Unpinned/private models should use
    /// [`from_candle_local`](Self::from_candle_local) directly.
    ///
    /// The load deliberately bypasses xberg's path-keyed backend cache: that cache
    /// can hold a backend built from earlier bytes at the same directory, so reusing
    /// it after verification would verify one set of weights and run another. A
    /// verified load always builds a fresh backend from the directory just verified,
    /// never from that cache.
    #[cfg(all(feature = "ner-candle", not(target_arch = "wasm32")))]
    pub fn from_candle_local_verified(
        model_dir: &std::path::Path,
        lora_adapter_dir: Option<&std::path::Path>,
        model_id: &str,
    ) -> Result<Self, PiiError> {
        let backend = verified_load(
            || {
                hacienda_model_assets::verify_dir(model_id, model_dir).map_err(|source| {
                    PiiError::ModelIntegrity {
                        model_id: model_id.to_owned(),
                        source,
                    }
                })
            },
            || {
                let backend = xberg::text::ner::candle::CandleBackend::from_local(
                    model_dir,
                    lora_adapter_dir,
                )
                .map_err(|source| PiiError::Ner {
                    message: format!(
                        "loading verified Candle GLiNER2 from {}",
                        model_dir.display()
                    ),
                    source: Box::new(source),
                })?;
                Ok(Arc::new(backend) as Arc<dyn NerBackend>)
            },
        )?;
        Ok(Self::with_runtime(
            backend,
            NerRuntimeState::loaded(Some(Arc::from(model_id))),
        ))
    }

    /// [`from_candle_local`](Self::from_candle_local)'s wasm32 sibling: loads the same
    /// Candle GLiNER2 backend from in-memory bytes rather than a filesystem model
    /// directory — a browser has no disk to read `model_dir` from. The caller (Studio)
    /// already fetches and IndexedDB-caches these three buffers for its own NER pass;
    /// this does not fetch anything itself.
    ///
    /// # Errors
    ///
    /// Returns [`PiiError::Ner`] if the model bytes cannot be loaded.
    #[cfg(all(target_arch = "wasm32", feature = "ner-candle-wasm"))]
    pub fn from_candle_bytes(
        weights: &[u8],
        tokenizer_json: &[u8],
        encoder_config_json: &[u8],
    ) -> Result<Self, PiiError> {
        let backend = xberg::text::ner::candle::CandleBackend::from_bytes(
            weights,
            tokenizer_json,
            encoder_config_json,
        )
        .map_err(|source| PiiError::Ner {
            message: "loading Candle GLiNER2 from in-memory bytes".into(),
            source: Box::new(source),
        })?;
        Ok(Self::with_runtime(
            Arc::new(backend),
            NerRuntimeState::loaded(None),
        ))
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

/// Verify, then load — in that order, with no path-keyed cache in between.
///
/// Kept as a small generic helper so the "verification binds the load" rule is
/// testable without real model weights: a failing verification must never reach
/// the loader, and the loader passed here is the uncached `from_local` path.
#[cfg(all(feature = "ner-candle", not(target_arch = "wasm32")))]
fn verified_load<E>(
    verify: impl FnOnce() -> Result<(), E>,
    load: impl FnOnce() -> Result<Arc<dyn NerBackend>, E>,
) -> Result<Arc<dyn NerBackend>, E> {
    verify()?;
    load()
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
            "access_token" => PiiCategory::SecretToken,
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

    #[test]
    fn runtime_state_tracks_the_detector_not_the_process() {
        let first = NerDetector::with_runtime(
            Arc::new(StubBackend(Vec::new())),
            NerRuntimeState::loaded(Some(Arc::from("org/first-model"))),
        );
        let second = NerDetector::with_runtime(
            Arc::new(StubBackend(Vec::new())),
            NerRuntimeState::loaded(Some(Arc::from("org/second-model"))),
        );

        // A second verified model must not be silently ignored in favour of the
        // first one loaded in this process.
        assert_eq!(
            first.runtime_state().model_id.as_deref(),
            Some("org/first-model")
        );
        assert_eq!(
            second.runtime_state().model_id.as_deref(),
            Some("org/second-model")
        );
        assert_ne!(first.runtime_state(), second.runtime_state());
    }

    #[test]
    fn unpinned_loaded_backend_is_model_backed_without_an_id() {
        let state = NerDetector::with_runtime(
            Arc::new(StubBackend(Vec::new())),
            NerRuntimeState::loaded(None),
        )
        .runtime_state();

        assert!(state.model_loaded, "private/unpinned model is still loaded");
        assert!(state.model_id.is_none());
    }

    #[test]
    fn a_plain_stub_backend_is_not_reported_as_model_loaded() {
        let state = detector(Vec::new()).runtime_state();
        assert!(!state.model_loaded);
        assert!(state.model_id.is_none());
    }

    #[cfg(all(feature = "ner-candle", not(target_arch = "wasm32")))]
    #[test]
    fn verified_load_never_reaches_the_loader_when_verification_fails() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        let loads = AtomicUsize::new(0);
        let result: Result<Arc<dyn NerBackend>, &'static str> = verified_load(
            || Err("digest mismatch"),
            || {
                loads.fetch_add(1, Ordering::SeqCst);
                Ok(Arc::new(StubBackend(Vec::new())) as Arc<dyn NerBackend>)
            },
        );

        assert_eq!(result.unwrap_err(), "digest mismatch");
        assert_eq!(
            loads.load(Ordering::SeqCst),
            0,
            "a failed verification must not load any backend"
        );
    }

    #[cfg(all(feature = "ner-candle", not(target_arch = "wasm32")))]
    #[tokio::test]
    async fn verified_load_builds_a_fresh_backend_after_verification() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        let steps = AtomicUsize::new(0);
        let mut verify_step = 0;
        let mut load_step = 0;
        let backend: Arc<dyn NerBackend> = verified_load(
            || {
                verify_step = steps.fetch_add(1, Ordering::SeqCst);
                Ok::<(), &'static str>(())
            },
            || {
                load_step = steps.fetch_add(1, Ordering::SeqCst);
                Ok(Arc::new(StubBackend(vec![entity(
                    EntityCategory::Person,
                    "Alice",
                    0,
                    Some(0.9),
                )])) as Arc<dyn NerBackend>)
            },
        )
        .unwrap();

        assert_eq!(verify_step, 0);
        assert_eq!(load_step, 1, "the uncached loader must run exactly once");
        assert_eq!(backend.detect("Alice").await.unwrap().len(), 1);
    }
}
