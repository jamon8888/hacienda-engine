//! Built-in PII detection patterns.

use super::types::{PatternMeta, PiiCategory};
use super::validators;

/// The deterministic pattern set the [`RegexEngine`](super::engine::RegexEngine) compiles.
///
/// Order matters: earlier patterns win when two matches overlap, because
/// `RegexEngine::find_all` sorts by start offset and drops later overlapping spans.
pub fn builtin_patterns() -> Vec<PatternMeta> {
    vec![
        // Email (RFC 5322 simplified)
        // Regex only — no identity can be confirmed from the shape, so 0.9
        // rather than 1.0 (calibrated against redact-paperasse's EMAIL_ADDRESS).
        PatternMeta::new(
            PiiCategory::Email,
            r"(?i)\b[a-z0-9._%+-]+@[a-z0-9.-]+\.[a-z]{2,}\b",
            false,
            "[EMAIL]",
        )
        // No `context_words`: a shape-only match must not be boosted to 1.0
        // by a nearby "email" label — that would claim verification that
        // never ran.
        .with_base_confidence(0.9),
        // French NIR (numéro de sécurité sociale). Checksum-validated with
        // INSEE mod-97, including Corsican 2A/2B department codes. Placed
        // before the generic digit-run patterns so a valid NIR is not stolen
        // by CreditCard or TaxId on an overlapping span.
        PatternMeta::new(
            PiiCategory::FrNir,
            // Separators include the ordinary space, NBSP (U+00A0) and the
            // narrow NBSP (U+202F) that French typesetting uses for numeric
            // grouping. INSEE allows 20-99 as an administrative month code
            // (unknown birth month) and 99 as a department (born abroad).
            // Leading registration codes 1-8 are all valid (1/2 are the
            // historical male/female codes).
            r"\b[1-8][ .\u{a0}\u{202f}]?\d{2}[ .\u{a0}\u{202f}]?(?:0[1-9]|1[0-2]|[2-9]\d)[ .\u{a0}\u{202f}]?(?:0[1-9]|[1-9]\d|2[AB])[ .\u{a0}\u{202f}]?\d{3}[ .\u{a0}\u{202f}]?\d{3}[ .\u{a0}\u{202f}]?\d{2}\b",
            true,
            "[NIR:****]",
        )
        .with_validator(validators::validate_fr_nir)
        .with_context(&["numéro", "sécurité", "sociale", "carte", "vitale", "nir", "insee"]),
        // French SIRET (14 digits) — Luhn plus a label or the canonical
        // 3-3-3-5 grouping. Registered before SIREN, though the word boundary
        // already stops SIREN from claiming the first nine digits of a SIRET.
        PatternMeta::new(
            PiiCategory::FrSiret,
            r"\b\d{3}[ .\u{a0}\u{202f}]?\d{3}[ .\u{a0}\u{202f}]?\d{3}[ .\u{a0}\u{202f}]?\d{5}\b",
            true,
            "[SIRET:****]",
        )
        .with_validator(validators::validate_fr_siret)
        .with_anchor(validators::anchor_fr_siret)
        .with_context(&["siret", "siren", "rcs", "euid", "établissement"]),
        // French SIREN (9 digits) — Luhn plus a *label*, never grouping alone.
        PatternMeta::new(
            PiiCategory::FrSiren,
            r"\b\d{3}[ .\u{a0}\u{202f}]?\d{3}[ .\u{a0}\u{202f}]?\d{3}\b",
            true,
            "[SIREN:****]",
        )
        .with_validator(validators::validate_fr_siren)
        .with_anchor(validators::anchor_fr_siren)
        .with_context(&["siren", "siret", "rcs", "euid", "société"]),
        // IBAN (general) — at least 15 characters after the country code. Track: precision
        // upgrade ported from presidio-rs (MIT) — a shape-only match here used to redact
        // any IBAN-looking string at full confidence; `validate_iban`'s ISO 13616 mod-97
        // check now promotes a real IBAN to 1.0 and discards one that fails the checksum,
        // and `base_confidence` (0.3) means a caller that skips detection but somehow
        // still sees this category treats it as medium, not certain, confidence.
        PatternMeta::new(
            PiiCategory::Iban,
            r"\b[A-Z]{2}\d{2}[A-Z0-9]{4}[A-Z0-9]{7,30}\b",
            true,
            "[IBAN:****]",
        )
        .with_base_confidence(0.3)
        .with_validator(validators::validate_iban)
        .with_context(&["iban", "bank", "account", "transfer", "swift"]),
        // SSN (US). `validate_us_ssn` only rejects structurally-impossible numbers (area
        // 000/666/9xx, group 00, serial 0000) — an SSN has no checksum that can positively
        // confirm it, so a plausible-but-unverified number keeps `base_confidence` (0.85,
        // calibrated against redact-paperasse's UsSsn) rather than being promoted to 1.0.
        PatternMeta::new(
            PiiCategory::Ssn,
            r"\b\d{3}-\d{2}-\d{4}\b",
            true,
            "[SSN:****]",
        )
        // No `context_words`: 0.85 + the +0.35 context boost would round up to
        // 1.0, which is reserved for a checksum that actually ran.
        .with_base_confidence(0.85)
        .with_validator(validators::validate_us_ssn),
        // Credit card: 13-16 digits with optional spaces or dashes. This shape alone
        // matches any 13-16 digit run (an invoice number, a tracking ID, ...).
        // `validate_credit_card` requires both a Luhn pass and an allocated issuer
        // prefix at a length that network issues — Luhn alone is a transcription
        // check that every Luhn-valid French SIRET also satisfies, so an SIRET used
        // to be redacted (and reported) as a card on every French invoice.
        PatternMeta::new(
            PiiCategory::CreditCard,
            // Greedy so grouped long PANs are captured whole; when it
            // absorbs a neighbouring amount, `retry_shorter` lets the engine
            // retry shorter digit-ending prefixes until Luhn+IIN accepts.
            r"\b(?:\d[ .-]?){12,18}\d\b",
            true,
            "[CARD:****]",
        )
        .with_base_confidence(0.3)
        .with_validator(validators::validate_credit_card)
        .with_retry_shorter()
        .with_context(&[
            "credit",
            "card",
            "visa",
            "mastercard",
            "amex",
            "discover",
            "diners",
            "maestro",
            "jcb",
            "cc",
        ]),
        // US routing number (must start with 0, 1, or 2)
        PatternMeta::new(
            PiiCategory::RoutingNumber,
            r"\b[0-2]\d{8}\b",
            true,
            "[ROUTING:****]",
        ),
        // SWIFT/BIC code (8 or 11 characters)
        PatternMeta::new(
            PiiCategory::SwiftBic,
            r"\b[A-Z]{4}[A-Z]{2}[A-Z0-9]{2}([A-Z0-9]{3})?\b",
            true,
            "[SWIFT:****]",
        ),
        // IPv4 address. L'ancre refuse une correspondance qui n'est que les
        // quatre premiers groupes d'une suite pointée plus longue — par
        // exemple un NIR français `2.96.01.12.345.678.59`.
        PatternMeta::new(
            PiiCategory::IpAddress,
            r"\b(?:\d{1,3}\.){3}\d{1,3}\b",
            false,
            "[IP]",
        )
        .with_anchor(validators::anchor_ipv4)
        .with_context(&["ip", "address", "ipv4", "ipv6"]),
        // IPv6 address (full form)
        PatternMeta::new(
            PiiCategory::IpAddress,
            r"\b(?:[0-9a-fA-F]{1,4}:){7}[0-9a-fA-F]{1,4}\b",
            false,
            "[IP]",
        )
        .with_context(&["ip", "address", "ipv4", "ipv6"]),
        // MAC address
        PatternMeta::new(
            PiiCategory::MacAddress,
            r"\b(?:[0-9a-fA-F]{2}[:-]){5}[0-9a-fA-F]{2}\b",
            false,
            "[MAC]",
        )
        .with_context(&["mac", "address"]),
        // URL
        PatternMeta::new(
            PiiCategory::Url,
            r#"(?i)\bhttps?://[^\s<>"']+"#,
            false,
            "[URL]",
        )
        .with_context(&["url", "link", "website", "site"]),
        // US passport number (letter followed by 8 digits)
        PatternMeta::new(
            PiiCategory::PassportNumber,
            r"\b[A-Z]\d{8}\b",
            true,
            "[PASSPORT:****]",
        ),
        // Date of birth (DD/MM/YYYY or DD.MM.YYYY)
        PatternMeta::new(
            PiiCategory::DateOfBirth,
            r"\b(?:0[1-9]|[12]\d|3[01])[/.](?:0[1-9]|1[0-2])[/.](?:19|20)\d{2}\b",
            false,
            "[DOB]",
        ),
        // Crypto wallet address (Bitcoin P2PKH or P2SH). `validate_crypto_wallet` verifies
        // the Base58Check double-SHA256 checksum every real address carries — a
        // Base58-shaped string that fails it is not a wallet address and is discarded
        // rather than redacted at full confidence.
        PatternMeta::new(
            PiiCategory::CryptoWallet,
            r"\b[13][a-km-zA-HJ-NP-Z1-9]{25,34}\b",
            false,
            "[WALLET:****]",
        )
        .with_base_confidence(0.5)
        .with_validator(validators::validate_crypto_wallet)
        .with_context(&["wallet", "btc", "bitcoin", "crypto", "address"]),
        // Medical record number
        PatternMeta::new(
            PiiCategory::MedicalRecordNumber,
            r"(?i)\bMRN[:\s]*\d{6,12}\b",
            true,
            "[MRN:****]",
        ),
        // US tax ID (EIN), XX-XXXXXXX
        PatternMeta::new(PiiCategory::TaxId, r"\b\d{2}-\d{7}\b", true, "[EIN:****]"),
        // Vehicle VIN: exactly 17 characters, excludes I, O, and Q. `validate_vin` checks
        // the ISO 3779 mod-11 check digit — any 17-char alphanumeric string used to match
        // this category at full confidence; now only a real (or non-North-American,
        // check-digit-less) VIN does.
        PatternMeta::new(
            PiiCategory::VehicleVin,
            r"\b[A-HJ-NPR-Z0-9]{17}\b",
            true,
            "[VIN:****]",
        )
        .with_base_confidence(0.5)
        .with_validator(validators::validate_vin)
        .with_context(&[
            "vin",
            "vehicle identification",
            "vehicle identification number",
            "chassis",
            "chassis number",
            "vehicle",
        ]),
        // API key
        PatternMeta::new(
            PiiCategory::ApiKey,
            r#"(?i)\b(?:api[_-]?key|apikey|api[_-]?secret)[:\s]*['"]?[a-zA-Z0-9_\-]{20,}['"]?"#,
            false,
            "[API_KEY]",
        ),
        // Secret token
        PatternMeta::new(
            PiiCategory::SecretToken,
            r#"(?i)\b(?:token|secret|password|passwd|pwd)[:\s]*['"]?[^\s'"]{8,}['"]?"#,
            false,
            "[SECRET]",
        ),
        // JWT
        PatternMeta::new(
            PiiCategory::JwtToken,
            r"\beyJ[a-zA-Z0-9_-]{10,}\.eyJ[a-zA-Z0-9_-]{10,}\.[a-zA-Z0-9_-]{10,}\b",
            false,
            "[JWT]",
        ),
        // EU VAT number (Track C2): a 2-letter member-state code (all 27, `EL` for
        // Greece) plus the country-specific body. The optional space / non-breaking
        // space after the code is how invoices typeset it; several member states
        // carry letters in the body, so the old digits-only shape missed them.
        // `validate_eu_vat` checksums FR/BE/NL/LU (promoting to 1.0), scores the
        // rest at the 0.8 base, and rejects a *failed* checksum outright.
        // Kept last so it never pre-empts a more specific category (IBAN, SWIFT, ...)
        // on an overlapping span.
        PatternMeta::new(
            PiiCategory::EuVat,
            r"(?i)\b(?:AT|BE|BG|CY|CZ|DE|DK|EE|EL|ES|FI|FR|HR|HU|IE|IT|LT|LU|LV|MT|NL|PL|PT|RO|SE|SI|SK)[ \u{00a0}]?[A-Z0-9]{2,13}\b",
            true,
            "[VAT:****]",
        )
        // No `context_words`: a nearby "TVA" label must not turn a format-only
        // 0.8 into 1.0. Only a verified check digit earns full confidence.
        .with_base_confidence(0.8)
        .with_validator(validators::validate_eu_vat),
        // Driver's license (Track C2): a letter followed by 7-13 digits. Deliberately
        // broad — license formats vary widely by jurisdiction — and placed last for the
        // same overlap-precedence reason as EU VAT above; mirrors the browser
        // detector's `driversLicense` regex.
        PatternMeta::new(
            PiiCategory::DriversLicense,
            r"\b[A-Z]\d{7,13}\b",
            true,
            "[LICENSE:****]",
        ),
        // Phone numbers are LAST on purpose. The regex only finds candidates
        // (international, parenthesised, and 7-15 digit runs with individually
        // optional separators — OCR merges them unpredictably), and a broad
        // digit-run candidate overlaps SSNs, SIRETs and NIRs. `find_all`
        // resolves equal spans by insertion order, so putting this last lets
        // the checksummed / structurally specific recognizers win and leaves
        // the phone category only for spans nobody else claims.
        //
        // No `context_words`: `context::enhance` adds +0.35 and would turn
        // 0.75 into 1.0, claiming more certainty than a format-only check
        // supports. The libphonenumber validator is the only opinion.
        PatternMeta::new(
            PiiCategory::PhoneNumber,
            concat!(
                r"(?:\+[1-9]\d{0,2}[\t .\u{a0}\u{202f}-]?\(?\d{1,2}\)?(?:[\t .\u{a0}\u{202f}-]?\d{1,2}){1,6})",
                r"|(?:\b0033[\t .\u{a0}\u{202f}-]?[1-9](?:[\t .\u{a0}\u{202f}-]?\d){8}\b)",
                r"|(?:\(\d{3}\)[\t .-]?\d{3}[\t .-]?\d{4})",
                r"|(?:\b\d{3}[.-]\d{3}[.-]\d{4}\b)",
                // National numbers: 10 or 11 digits beginning with 0 (UK
                // `020 7183 8750` has 11), with individually optional
                // separators (OCR merges them). The lazy quantifier prefers
                // the shorter span so adjacent numbers stay separate:
                // `01 56 35 91 80 06 12 34 56 78` yields two entities.
                r"|(?:\b0[1-9](?:[\t .\u{a0}\u{202f}-]?\d){8,9}?\b)",
            ),
            false,
            "[PHONE]",
        )
        .with_base_confidence(0.75)
        .with_validator(validators::validate_phone_number),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn should_compile_every_builtin_pattern() {
        for meta in builtin_patterns() {
            regex::Regex::new(&meta.pattern).unwrap_or_else(|e| {
                panic!("pattern for {:?} does not compile: {e}", meta.category)
            });
        }
    }

    #[test]
    fn should_give_every_pattern_a_non_empty_redact_template() {
        for meta in builtin_patterns() {
            assert!(
                !meta.redact_template.is_empty(),
                "{:?} has an empty redact template",
                meta.category
            );
        }
    }

    #[test]
    fn should_give_every_pattern_a_confidence_in_range() {
        for meta in builtin_patterns() {
            assert!(
                (0.0..=1.0).contains(&meta.base_confidence),
                "{:?} has base_confidence {} outside [0, 1]",
                meta.category,
                meta.base_confidence
            );
        }
    }
}
