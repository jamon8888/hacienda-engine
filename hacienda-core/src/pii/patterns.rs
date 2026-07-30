//! Built-in PII detection patterns.

use super::types::{PatternMeta, PiiCategory};

/// The deterministic pattern set the [`RegexEngine`](super::engine::RegexEngine) compiles.
///
/// Order matters: earlier patterns win when two matches overlap, because
/// `RegexEngine::find_all` sorts by start offset and drops later overlapping spans.
pub fn builtin_patterns() -> Vec<PatternMeta> {
    vec![
        // Email (RFC 5322 simplified)
        PatternMeta {
            category: PiiCategory::Email,
            pattern: r"(?i)\b[a-z0-9._%+-]+@[a-z0-9.-]+\.[a-z]{2,}\b".into(),
            format_preserving: false,
            redact_template: "[EMAIL]".into(),
        },
        // Phone international. Requires a `+` prefix or parenthesised area code so this
        // does not swallow bare credit-card digit runs.
        PatternMeta {
            category: PiiCategory::PhoneNumber,
            pattern: r"(?:\+[1-9]\d{0,2}[-.\s]?\(?\d{1,4}\)?[-.\s]?\d{1,4}[-.\s]?\d{1,9})|(?:\(\d{2,4}\)[-\s]?\d{3,4}[-.\s]?\d{3,4})".into(),
            format_preserving: false,
            redact_template: "[PHONE]".into(),
        },
        // Phone France
        PatternMeta {
            category: PiiCategory::PhoneNumber,
            pattern: r"(?:\+33|0033)[ .-]?[1-9](?:[ .-]?\d{2}){4}".into(),
            format_preserving: false,
            redact_template: "[PHONE]".into(),
        },
        // IBAN (general) — at least 15 characters after the country code
        PatternMeta {
            category: PiiCategory::Iban,
            pattern: r"\b[A-Z]{2}\d{2}[A-Z0-9]{4}[A-Z0-9]{7,30}\b".into(),
            format_preserving: true,
            redact_template: "[IBAN:****]".into(),
        },
        // SSN (US)
        PatternMeta {
            category: PiiCategory::Ssn,
            pattern: r"\b\d{3}-\d{2}-\d{4}\b".into(),
            format_preserving: true,
            redact_template: "[SSN:****]".into(),
        },
        // Credit card: 13-16 digits with optional spaces or dashes
        PatternMeta {
            category: PiiCategory::CreditCard,
            pattern: r"\b(?:\d[ -]*?){13,16}\b".into(),
            format_preserving: true,
            redact_template: "[CARD:****]".into(),
        },
        // US routing number (must start with 0, 1, or 2)
        PatternMeta {
            category: PiiCategory::RoutingNumber,
            pattern: r"\b[0-2]\d{8}\b".into(),
            format_preserving: true,
            redact_template: "[ROUTING:****]".into(),
        },
        // SWIFT/BIC code (8 or 11 characters)
        PatternMeta {
            category: PiiCategory::SwiftBic,
            pattern: r"\b[A-Z]{4}[A-Z]{2}[A-Z0-9]{2}([A-Z0-9]{3})?\b".into(),
            format_preserving: true,
            redact_template: "[SWIFT:****]".into(),
        },
        // IPv4 address
        PatternMeta {
            category: PiiCategory::IpAddress,
            pattern: r"\b(?:\d{1,3}\.){3}\d{1,3}\b".into(),
            format_preserving: false,
            redact_template: "[IP]".into(),
        },
        // IPv6 address (full form)
        PatternMeta {
            category: PiiCategory::IpAddress,
            pattern: r"\b(?:[0-9a-fA-F]{1,4}:){7}[0-9a-fA-F]{1,4}\b".into(),
            format_preserving: false,
            redact_template: "[IP]".into(),
        },
        // MAC address
        PatternMeta {
            category: PiiCategory::MacAddress,
            pattern: r"\b(?:[0-9a-fA-F]{2}[:-]){5}[0-9a-fA-F]{2}\b".into(),
            format_preserving: false,
            redact_template: "[MAC]".into(),
        },
        // URL
        PatternMeta {
            category: PiiCategory::Url,
            pattern: r#"(?i)\bhttps?://[^\s<>"']+"#.into(),
            format_preserving: false,
            redact_template: "[URL]".into(),
        },
        // US passport number (letter followed by 8 digits)
        PatternMeta {
            category: PiiCategory::PassportNumber,
            pattern: r"\b[A-Z]\d{8}\b".into(),
            format_preserving: true,
            redact_template: "[PASSPORT:****]".into(),
        },
        // Date of birth (DD/MM/YYYY or DD.MM.YYYY)
        PatternMeta {
            category: PiiCategory::DateOfBirth,
            pattern: r"\b(?:0[1-9]|[12]\d|3[01])[/.](?:0[1-9]|1[0-2])[/.](?:19|20)\d{2}\b".into(),
            format_preserving: false,
            redact_template: "[DOB]".into(),
        },
        // Crypto wallet address (Bitcoin P2PKH or P2SH)
        PatternMeta {
            category: PiiCategory::CryptoWallet,
            pattern: r"\b[13][a-km-zA-HJ-NP-Z1-9]{25,34}\b".into(),
            format_preserving: false,
            redact_template: "[WALLET:****]".into(),
        },
        // Medical record number
        PatternMeta {
            category: PiiCategory::MedicalRecordNumber,
            pattern: r"(?i)\bMRN[:\s]*\d{6,12}\b".into(),
            format_preserving: true,
            redact_template: "[MRN:****]".into(),
        },
        // US tax ID (EIN), XX-XXXXXXX
        PatternMeta {
            category: PiiCategory::TaxId,
            pattern: r"\b\d{2}-\d{7}\b".into(),
            format_preserving: true,
            redact_template: "[EIN:****]".into(),
        },
        // Vehicle VIN: exactly 17 characters, excludes I, O, and Q
        PatternMeta {
            category: PiiCategory::VehicleVin,
            pattern: r"\b[A-HJ-NPR-Z0-9]{17}\b".into(),
            format_preserving: true,
            redact_template: "[VIN:****]".into(),
        },
        // API key
        PatternMeta {
            category: PiiCategory::ApiKey,
            pattern: r#"(?i)\b(?:api[_-]?key|apikey|api[_-]?secret)[:\s]*['"]?[a-zA-Z0-9_\-]{20,}['"]?"#.into(),
            format_preserving: false,
            redact_template: "[API_KEY]".into(),
        },
        // Secret token
        PatternMeta {
            category: PiiCategory::SecretToken,
            pattern: r#"(?i)\b(?:token|secret|password|passwd|pwd)[:\s]*['"]?[^\s'"]{8,}['"]?"#.into(),
            format_preserving: false,
            redact_template: "[SECRET]".into(),
        },
        // JWT
        PatternMeta {
            category: PiiCategory::JwtToken,
            pattern: r"\beyJ[a-zA-Z0-9_-]{10,}\.eyJ[a-zA-Z0-9_-]{10,}\.[a-zA-Z0-9_-]{10,}\b".into(),
            format_preserving: false,
            redact_template: "[JWT]".into(),
        },
        // EU VAT number (Track C2): a 2-letter member-state code (all 27, `EL` for
        // Greece) plus 2-12 digits. Placed last so it never pre-empts a more specific
        // category (IBAN, SWIFT, ...) on an overlapping span — mirrors the browser
        // detector's `euVat` regex, the pattern this fills the gap left by.
        PatternMeta {
            category: PiiCategory::EuVat,
            pattern: r"(?i)\b(?:AT|BE|BG|CY|CZ|DE|DK|EE|EL|ES|FI|FR|HR|HU|IE|IT|LT|LU|LV|MT|NL|PL|PT|RO|SE|SI|SK)\d{2,12}\b".into(),
            format_preserving: true,
            redact_template: "[VAT:****]".into(),
        },
        // Driver's license (Track C2): a letter followed by 7-13 digits. Deliberately
        // broad — license formats vary widely by jurisdiction — and placed last for the
        // same overlap-precedence reason as EU VAT above; mirrors the browser
        // detector's `driversLicense` regex.
        PatternMeta {
            category: PiiCategory::DriversLicense,
            pattern: r"\b[A-Z]\d{7,13}\b".into(),
            format_preserving: true,
            redact_template: "[LICENSE:****]".into(),
        },
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
}
