//! Deterministic regex-based PII detection.

use super::patterns::builtin_patterns;
use super::types::{PatternMeta, RegexEntity};
use crate::pii::PiiError;
use regex::Regex;

/// Compiles and runs the built-in PII patterns over a text.
///
/// Patterns are compiled eagerly in [`RegexEngine::new`] so a malformed pattern
/// surfaces as an error at construction rather than silently matching nothing.
pub struct RegexEngine {
    patterns: Vec<PatternMeta>,
    compiled: Vec<Regex>,
}

impl RegexEngine {
    /// Compile the built-in pattern set.
    ///
    /// # Errors
    ///
    /// Returns [`PiiError::Pattern`] if any built-in pattern fails to compile.
    pub fn new() -> Result<Self, PiiError> {
        Self::with_patterns(builtin_patterns())
    }

    /// Compile a caller-supplied pattern set.
    ///
    /// # Errors
    ///
    /// Returns [`PiiError::Pattern`] if any pattern fails to compile.
    pub fn with_patterns(patterns: Vec<PatternMeta>) -> Result<Self, PiiError> {
        let compiled = patterns
            .iter()
            .map(|meta| {
                Regex::new(&meta.pattern).map_err(|source| PiiError::Pattern {
                    category: meta.category.to_string(),
                    source,
                })
            })
            .collect::<Result<Vec<_>, _>>()?;

        Ok(Self { patterns, compiled })
    }

    /// Find every non-overlapping PII span in `text`, ordered by start offset.
    ///
    /// When two patterns match overlapping spans the earlier-starting match wins;
    /// ties are broken by the pattern order in [`builtin_patterns`].
    ///
    /// A pattern with a [`PatternMeta::validator`] has its matched text checked before
    /// being kept: `Some(false)` drops the match entirely (e.g. a 16-digit run that fails
    /// Luhn is not a credit card), `Some(true)` promotes its confidence to `1.0`, and
    /// `None` keeps [`PatternMeta::base_confidence`] as-is. Patterns with no validator are
    /// unaffected — same `base_confidence` (`1.0` unless calibrated lower) every match.
    pub fn find_all(&self, text: &str) -> Vec<RegexEntity> {
        let mut entities = Vec::new();

        for (meta, re) in self.patterns.iter().zip(&self.compiled) {
            for m in re.find_iter(text) {
                let confidence = match meta.validator {
                    Some(validate) => match validate(m.as_str()) {
                        Some(false) => continue,
                        Some(true) => 1.0,
                        None => meta.base_confidence,
                    },
                    None => meta.base_confidence,
                };
                entities.push(RegexEntity {
                    category: meta.category.clone(),
                    start: m.start() as u32,
                    end: m.end() as u32,
                    confidence,
                    format_preserving: meta.format_preserving,
                    redact_template: meta.redact_template.clone(),
                    context_words: meta.context_words,
                });
            }
        }

        entities.sort_by(|a, b| a.start.cmp(&b.start).then(a.end.cmp(&b.end)));

        let mut deduped: Vec<RegexEntity> = Vec::with_capacity(entities.len());
        for entity in entities {
            if deduped.last().is_some_and(|last| entity.start < last.end) {
                continue;
            }
            deduped.push(entity);
        }

        deduped
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pii::types::PiiCategory;

    fn categories(text: &str) -> Vec<PiiCategory> {
        RegexEngine::new()
            .expect("built-in patterns compile")
            .find_all(text)
            .into_iter()
            .map(|e| e.category)
            .collect()
    }

    #[test]
    fn should_detect_an_email_address() {
        assert_eq!(
            categories("write to alice@example.com now"),
            vec![PiiCategory::Email]
        );
    }

    #[test]
    fn should_detect_a_us_ssn() {
        assert_eq!(categories("SSN 123-45-6789"), vec![PiiCategory::Ssn]);
    }

    #[test]
    fn should_detect_an_ipv4_address() {
        assert_eq!(
            categories("host 192.168.1.10"),
            vec![PiiCategory::IpAddress]
        );
    }

    #[test]
    fn should_detect_a_real_credit_card_number() {
        assert_eq!(
            categories("charged to 4111111111111111 today"),
            vec![PiiCategory::CreditCard]
        );
    }

    #[test]
    fn should_not_flag_a_luhn_invalid_number_as_a_credit_card() {
        // Shape-only, this would have matched `CreditCard`'s pattern before checksum
        // validation existed -- a random 16-digit number (invoice, tracking ID, ...) is
        // not a credit card, and Luhn confirms it.
        assert!(categories("invoice number 1234567890123456").is_empty());
    }

    #[test]
    fn should_promote_a_validated_credit_card_to_full_confidence() {
        let entities = RegexEngine::new()
            .unwrap()
            .find_all("card 4111111111111111 on file");
        assert_eq!(entities.len(), 1);
        assert_eq!(entities[0].confidence, 1.0);
    }

    #[test]
    fn should_detect_a_real_iban() {
        assert_eq!(
            categories("transfer to GB82WEST12345698765432"),
            vec![PiiCategory::Iban]
        );
    }

    #[test]
    fn should_not_flag_an_iban_shaped_string_that_fails_its_checksum() {
        assert!(categories("reference GB00WEST12345698765432").is_empty());
    }

    #[test]
    fn should_detect_a_valid_vin() {
        assert_eq!(
            categories("vehicle 1M8GDM9AXKP042788 registered"),
            vec![PiiCategory::VehicleVin]
        );
    }

    #[test]
    fn should_not_flag_a_north_american_vin_with_a_bad_check_digit() {
        assert!(categories("code 12111111111111111 on the form").is_empty());
    }

    #[test]
    fn should_detect_a_valid_crypto_wallet_address() {
        assert_eq!(
            categories("send to 1BvBMSEYstWetqTFn5Au4m4GFg7xJaNVN2 please"),
            vec![PiiCategory::CryptoWallet]
        );
    }

    #[test]
    fn should_not_flag_a_wallet_shaped_string_with_a_bad_checksum() {
        assert!(categories("id 1BvBMSEYstWetqTFn5Au4m4GFg7xJaNVN3 unknown").is_empty());
    }

    #[test]
    fn should_keep_a_structurally_plausible_ssn_at_medium_confidence_without_context() {
        let entities = RegexEngine::new()
            .unwrap()
            .find_all("reference 123-45-6789 filed");
        assert_eq!(entities.len(), 1);
        assert_eq!(entities[0].confidence, 0.4);
    }

    #[test]
    fn should_not_flag_a_structurally_impossible_ssn() {
        // Area 000 can never be a real SSN.
        assert!(categories("code 000-45-6789 on file").is_empty());
    }

    #[test]
    fn should_detect_an_eu_vat_number() {
        // Track C2: the French client base's gap. FR + 11 digits is a real French VAT
        // shape (2-digit key + SIREN); the pattern itself accepts any EU member code.
        assert_eq!(
            categories("VAT FR12345678901 on the invoice"),
            vec![PiiCategory::EuVat]
        );
    }

    #[test]
    fn should_detect_a_greek_vat_number_under_its_el_prefix() {
        // Greece's ISO country code is GR, but its VAT prefix is EL -- the one
        // deliberate exception in the 27-code list, carried over from the browser
        // detector's regex rather than re-derived, since getting this wrong silently
        // stops matching a real client's VAT numbers.
        assert_eq!(categories("EL123456789"), vec![PiiCategory::EuVat]);
    }

    #[test]
    fn should_detect_a_drivers_license_number() {
        assert_eq!(
            categories("license A1234567 on file"),
            vec![PiiCategory::DriversLicense]
        );
    }

    #[test]
    fn should_return_spans_that_slice_the_source_text() {
        let text = "contact bob@corp.io please";
        let entities = RegexEngine::new().unwrap().find_all(text);
        assert_eq!(entities.len(), 1);
        let e = &entities[0];
        assert_eq!(&text[e.start as usize..e.end as usize], "bob@corp.io");
    }

    #[test]
    fn should_return_entities_sorted_and_non_overlapping() {
        let text = "a@b.com then 123-45-6789 then 10.0.0.1";
        let entities = RegexEngine::new().unwrap().find_all(text);
        assert!(entities.len() >= 2);
        for pair in entities.windows(2) {
            assert!(pair[0].end <= pair[1].start, "spans overlap: {pair:?}");
        }
    }

    #[test]
    fn should_find_nothing_in_text_without_pii() {
        assert!(categories("the quick brown fox").is_empty());
    }

    #[test]
    fn should_reject_a_pattern_that_does_not_compile() {
        let bad = vec![PatternMeta::new(
            PiiCategory::Custom("broken".into()),
            "(",
            false,
            "[X]",
        )];
        assert!(matches!(
            RegexEngine::with_patterns(bad),
            Err(PiiError::Pattern { .. })
        ));
    }
}
