use hacienda_core::pii::ner::to_pii_category;
use hacienda_core::pii::types::PiiCategory;
use xberg::types::entity::EntityCategory;

#[test]
fn should_map_all_42_privacy_filter_labels_to_pii_category() {
    let cases: &[(&str, PiiCategory)] = &[
        ("person", PiiCategory::Person),
        ("full_name", PiiCategory::FullName),
        ("first_name", PiiCategory::FirstName),
        ("middle_name", PiiCategory::MiddleName),
        ("last_name", PiiCategory::LastName),
        ("date_of_birth", PiiCategory::DateOfBirth),
        ("email", PiiCategory::Email),
        ("phone_number", PiiCategory::PhoneNumber),
        ("address", PiiCategory::Address),
        ("street_address", PiiCategory::StreetAddress),
        ("city", PiiCategory::City),
        ("state_or_region", PiiCategory::StateOrRegion),
        ("postal_code", PiiCategory::PostalCode),
        ("country", PiiCategory::Country),
        ("government_id", PiiCategory::GovernmentId),
        ("national_id_number", PiiCategory::NationalId),
        ("passport_number", PiiCategory::PassportNumber),
        ("drivers_license_number", PiiCategory::DriversLicense),
        ("license_number", PiiCategory::DriversLicense),
        ("tax_id", PiiCategory::TaxId),
        ("tax_number", PiiCategory::TaxId),
        ("bank_account", PiiCategory::BankAccount),
        ("account_number", PiiCategory::BankAccount),
        ("routing_number", PiiCategory::RoutingNumber),
        ("iban", PiiCategory::Iban),
        ("payment_card", PiiCategory::PaymentCard),
        ("card_number", PiiCategory::CreditCard),
        ("card_expiry", PiiCategory::CardExpiry),
        ("card_cvv", PiiCategory::CardCvv),
        ("username", PiiCategory::Username),
        ("ip_address", PiiCategory::IpAddress),
        ("password", PiiCategory::Password),
        ("secret", PiiCategory::SecretToken),
        ("api_key", PiiCategory::ApiKey),
        ("access_token", PiiCategory::SecretToken),
        // 7 left map to Custom collapse — assert that too:
        ("account_id", PiiCategory::Custom("account_id".into())),
        (
            "sensitive_account_id",
            PiiCategory::Custom("sensitive_account_id".into()),
        ),
        ("recovery_code", PiiCategory::Custom("recovery_code".into())),
        (
            "sensitive_date",
            PiiCategory::Custom("sensitive_date".into()),
        ),
        ("document_date", PiiCategory::Custom("document_date".into())),
        (
            "expiration_date",
            PiiCategory::Custom("expiration_date".into()),
        ),
        (
            "transaction_date",
            PiiCategory::Custom("transaction_date".into()),
        ),
    ];
    for (wire, expected) in cases {
        let cat = to_pii_category(&EntityCategory::Custom(wire.to_string()));
        assert_eq!(
            &cat, expected,
            "wire {wire:?} mapped to {cat:?} not {expected:?}"
        );
    }
}

#[test]
fn should_size_comprehensive_to_41() {
    use hacienda_core::pii::config::VerticalConfig;
    assert_eq!(VerticalConfig::comprehensive().labels.len(), 41);
}

#[test]
fn should_apply_per_category_threshold_offsets() {
    use hacienda_core::pii::config::PipelineConfig;
    use hacienda_core::pii::types::PiiCategory;
    let cfg = PipelineConfig::default();
    assert_eq!(cfg.effective_threshold(&PiiCategory::FirstName), 0.65);
    assert_eq!(cfg.effective_threshold(&PiiCategory::LastName), 0.65);
    assert_eq!(cfg.effective_threshold(&PiiCategory::Person), 0.65);
    assert_eq!(cfg.effective_threshold(&PiiCategory::Email), 0.48);
    assert_eq!(cfg.effective_threshold(&PiiCategory::Iban), 0.48);
    assert_eq!(cfg.effective_threshold(&PiiCategory::TaxId), 0.50);
}
