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
        PatternMeta::new(
            PiiCategory::Email,
            r"(?i)\b[a-z0-9._%+-]+@[a-z0-9.-]+\.[a-z]{2,}\b",
            false,
            "[EMAIL]",
        )
        .with_context(&["email", "mail", "e-mail"]),
        // Phone international. Requires a `+` prefix or parenthesised area code so this
        // does not swallow bare credit-card digit runs.
        PatternMeta::new(
            PiiCategory::PhoneNumber,
            r"(?:\+[1-9]\d{0,2}[-.\s]?\(?\d{1,4}\)?[-.\s]?\d{1,4}[-.\s]?\d{1,9})|(?:\(\d{2,4}\)[-\s]?\d{3,4}[-.\s]?\d{3,4})",
            false,
            "[PHONE]",
        ),
        // Phone France
        PatternMeta::new(
            PiiCategory::PhoneNumber,
            r"(?:\+33|0033)[ .-]?[1-9](?:[ .-]?\d{2}){4}",
            false,
            "[PHONE]",
        ),
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
        // confirm it, so a plausible-but-unverified number keeps `base_confidence` (0.4)
        // rather than being promoted to 1.0.
        PatternMeta::new(
            PiiCategory::Ssn,
            r"\b\d{3}-\d{2}-\d{4}\b",
            true,
            "[SSN:****]",
        )
        .with_base_confidence(0.4)
        .with_validator(validators::validate_us_ssn)
        .with_context(&["social", "security", "ssn", "ssns", "ssid"]),
        // Credit card: 13-16 digits with optional spaces or dashes. This shape alone
        // matches any 13-16 digit run (an invoice number, a tracking ID, ...); `luhn_valid`
        // via `validate_credit_card` is what actually distinguishes a real card number,
        // discarding anything that fails the check instead of redacting it at full
        // confidence like every other category still does.
        PatternMeta::new(
            PiiCategory::CreditCard,
            r"\b(?:\d[ -]*?){13,16}\b",
            true,
            "[CARD:****]",
        )
        .with_base_confidence(0.3)
        .with_validator(validators::validate_credit_card)
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
        // IPv4 address
        PatternMeta::new(
            PiiCategory::IpAddress,
            r"\b(?:\d{1,3}\.){3}\d{1,3}\b",
            false,
            "[IP]",
        )
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
        // Greece) plus 2-12 digits. Placed last so it never pre-empts a more specific
        // category (IBAN, SWIFT, ...) on an overlapping span — mirrors the browser
        // detector's `euVat` regex, the pattern this fills the gap left by.
        PatternMeta::new(
            PiiCategory::EuVat,
            r"(?i)\b(?:AT|BE|BG|CY|CZ|DE|DK|EE|EL|ES|FI|FR|HR|HU|IE|IT|LT|LU|LV|MT|NL|PL|PT|RO|SE|SI|SK)\d{2,12}\b",
            true,
            "[VAT:****]",
        ),
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
