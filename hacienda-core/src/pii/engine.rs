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
                if let Some(anchor) = meta.anchor {
                    if !anchor(text, m.start() as u32, m.as_str()) {
                        continue;
                    }
                }
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
    fn should_capture_full_long_and_grouped_pan_spans() {
        let engine = RegexEngine::new().unwrap();
        for (texte, pan) in [
            ("carte 4000000000000000006 utilisée", "4000000000000000006"),
            (
                "carte 4000 0000 0000 0000 006 utilisée",
                "4000 0000 0000 0000 006",
            ),
            ("carte 3050 0000 0000 0003 utilisée", "3050 0000 0000 0003"),
        ] {
            let entites = engine.find_all(texte);
            let carte = entites
                .iter()
                .find(|e| e.category == PiiCategory::CreditCard)
                .unwrap_or_else(|| panic!("PAN manqué dans {texte:?}"));
            assert_eq!(&texte[carte.start as usize..carte.end as usize], pan);
            assert_eq!(carte.confidence, 1.0);
        }
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
    fn should_keep_a_structurally_plausible_ssn_below_full_confidence_without_context() {
        // No checksum exists for an SSN; structural exclusions make 0.85 an
        // honest claim but not certainty (which is reserved for real checksums).
        let entities = RegexEngine::new()
            .unwrap()
            .find_all("reference 123-45-6789 filed");
        assert_eq!(entities.len(), 1);
        assert_eq!(entities[0].confidence, 0.85);
    }

    #[test]
    fn should_not_flag_a_structurally_impossible_ssn() {
        // Area 000 can never be a real SSN.
        assert!(categories("code 000-45-6789 on file").is_empty());
    }

    #[test]
    fn should_detect_a_checksum_valid_french_vat_number() {
        // Track C2: the French client base's gap. FR + 11 digits is a real French VAT
        // shape (2-digit key + SIREN). `FR40303265045` carries key 40 over SIREN
        // 303265045, which satisfies the mod-97 relation.
        let entities = RegexEngine::new()
            .unwrap()
            .find_all("VAT FR40303265045 on the invoice");
        assert_eq!(
            entities.iter().map(|e| &e.category).collect::<Vec<_>>(),
            vec![&PiiCategory::EuVat]
        );
        assert_eq!(entities[0].confidence, 1.0);
    }

    #[test]
    fn should_reject_an_eu_vat_with_a_bad_checksum() {
        // Same shape, wrong key: a failed check is stronger evidence than an
        // unrun one, so the match is discarded rather than scored at 0.8.
        assert!(categories("VAT FR41303265045 on the invoice").is_empty());
    }

    #[test]
    fn should_keep_a_format_only_vat_at_reduced_confidence() {
        // Germany has no public check digit; 0.8 is an honest claim, not 1.0.
        let entities = RegexEngine::new()
            .unwrap()
            .find_all("VAT DE123456789 on file");
        assert_eq!(entities.len(), 1);
        assert_eq!(entities[0].confidence, 0.8);
    }

    #[test]
    fn should_detect_a_labelled_siret() {
        let entities = RegexEngine::new()
            .unwrap()
            .find_all("SIRET 732 829 320 00074 on the invoice");
        assert!(entities
            .iter()
            .any(|e| e.category == PiiCategory::FrSiret && e.confidence == 1.0));
    }

    #[test]
    fn should_detect_an_unlabelled_but_canonically_grouped_siret() {
        // Verbatim shape from a real CFE form: the nearest textual label was
        // 2,758 characters away on a later page, so the grouping is the anchor.
        let entities = RegexEngine::new()
            .unwrap()
            .find_all("75008 PARIS 944 681 634 00019 66198");
        assert!(entities.iter().any(|e| e.category == PiiCategory::FrSiret));
    }

    #[test]
    fn should_detect_a_labelled_siren_but_not_a_grouped_amount() {
        let labelled = RegexEngine::new().unwrap().find_all("SIREN 732 829 320");
        assert!(labelled.iter().any(|e| e.category == PiiCategory::FrSiren));

        // `732 829 320` passes Luhn and is how French typesets millions.
        // Without a label it must not become a SIREN (other categories may
        // legitimately claim the span, e.g. a phone-shaped candidate).
        let amount = RegexEngine::new()
            .unwrap()
            .find_all("montant : 732 829 320");
        assert!(!amount
            .iter()
            .any(|e| e.category == PiiCategory::FrSiren || e.category == PiiCategory::FrSiret));
    }

    #[test]
    fn should_not_detect_an_unlabelled_compact_siret() {
        // Luhn alone is not evidence: without a label or canonical grouping
        // this is just a 14-digit number passing a checksum.
        let entities = RegexEngine::new()
            .unwrap()
            .find_all("référence 73282932000074");
        assert!(!entities.iter().any(|e| e.category == PiiCategory::FrSiret));
    }

    #[test]
    fn should_detect_a_checksum_valid_french_nir() {
        let entities = RegexEngine::new()
            .unwrap()
            .find_all("numéro de sécurité sociale 1 85 01 75 123 456 09");
        assert_eq!(
            entities.iter().map(|e| &e.category).collect::<Vec<_>>(),
            vec![&PiiCategory::FrNir]
        );
        assert_eq!(entities[0].confidence, 1.0);
    }

    #[test]
    fn should_not_flag_a_nir_shaped_number_with_a_bad_checksum() {
        assert!(categories("numéro 291059933807693 on file").is_empty());
    }

    #[test]
    fn should_reject_a_checksum_valid_nir_with_an_impossible_month() {
        // La clé mod-97 est bonne (26) : seul le contrôle du mois peut rejeter.
        assert!(categories("numéro 291139933807626 on file").is_empty());
    }

    #[test]
    fn should_detect_nir_with_extended_registration_codes_and_narrow_spaces() {
        for texte in [
            "numéro 385017512345606 déclaré",
            "numéro 885017512345647 déclaré",
            "numéro 1\u{202f}85\u{202f}01\u{202f}75\u{202f}123\u{202f}456\u{202f}09 déclaré",
        ] {
            assert!(
                categories(texte).contains(&PiiCategory::FrNir),
                "NIR valide manqué : {texte:?}"
            );
        }
    }

    #[test]
    fn should_detect_a_french_national_phone_number() {
        // The gap that motivated the libphonenumber switch: every previous
        // pattern required `+`, parentheses or a `+33` prefix, so the way
        // French people actually write their number sailed through.
        let entities = RegexEngine::new()
            .unwrap()
            .find_all("téléphone : 01 56 35 91 80");
        assert_eq!(
            entities.iter().map(|e| &e.category).collect::<Vec<_>>(),
            vec![&PiiCategory::PhoneNumber]
        );
        assert_eq!(entities[0].confidence, 0.75);
    }

    #[test]
    fn should_detect_an_ocr_merged_french_phone_number() {
        // Verbatim from a real form: OCR merged the pair separators.
        assert_eq!(
            categories("téléphonez au : 0156 35 9180"),
            vec![PiiCategory::PhoneNumber]
        );
    }

    #[test]
    fn should_detect_two_adjacent_french_numbers_without_merging_them() {
        let texte = "01 56 35 91 80 06 12 34 56 78";
        let telephone: Vec<_> = RegexEngine::new()
            .unwrap()
            .find_all(texte)
            .into_iter()
            .filter(|e| e.category == PiiCategory::PhoneNumber)
            .map(|e| &texte[e.start as usize..e.end as usize])
            .collect();
        assert_eq!(
            telephone,
            vec!["01 56 35 91 80", "06 12 34 56 78"],
            "deux numéros adjacents doivent produire deux entités"
        );
    }

    #[test]
    fn should_preserve_an_explicit_0033_prefix() {
        assert!(categories("appel 0033612345678").contains(&PiiCategory::PhoneNumber));
    }

    #[test]
    fn should_accept_tabs_and_narrow_no_break_spaces_as_separators() {
        for texte in [
            "appel 01\t56\t35\t91\t80",
            "appel 01\u{202f}56\u{202f}35\u{202f}91\u{202f}80",
            "appel +33\u{202f}6\u{202f}12\u{202f}34\u{202f}56\u{202f}78",
        ] {
            assert!(
                categories(texte).contains(&PiiCategory::PhoneNumber),
                "téléphone manqué : {texte:?}"
            );
        }
    }

    #[test]
    fn should_not_flag_a_siret_as_a_phone_number() {
        // libphonenumber would pick `00074` out of this; candidate-span
        // validation rejects it because no match covers the whole span. The
        // SIRET itself is still detected by its own anchored recognizer.
        let entities = RegexEngine::new()
            .unwrap()
            .find_all("SIRET 732 829 320 00074");
        assert!(!entities
            .iter()
            .any(|e| e.category == PiiCategory::PhoneNumber));
        assert!(entities.iter().any(|e| e.category == PiiCategory::FrSiret));
    }

    #[test]
    fn should_not_flag_a_compact_digit_run_as_a_phone_number() {
        // Even though libphonenumber accepts `0156359180`, a bare digit run is
        // too ambiguous with account/order numbers by policy.
        assert!(categories("reference 0156359180 filed").is_empty());
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
