//! Checksum / structural validators used to promote or reject a regex match.
//!
//! Ported from [presidio-rs](https://github.com/jqueguiner/presidio-rs)'s
//! `crates/presidio-analyzer/src/validators.rs` (MIT License, Copyright (c) 2026
//! presidio-rust contributors), a Rust port of Microsoft Presidio. Adapted to this
//! crate's contract: [`RegexEngine`](super::engine::RegexEngine) calls a validator after
//! a pattern matches, and treats the result as:
//!
//!  * `Some(true)`  — validated; confidence is promoted to `1.0`
//!  * `Some(false)` — invalid; the match is discarded entirely
//!  * `None`        — no opinion; the pattern's own `base_confidence` is kept as-is
//!
//! `RegexEngine::find_all` hardcoded every match to `confidence: 1.0` before this module
//! existed — a 16-digit number that fails Luhn is very unlikely to be a real credit card,
//! but was flagged (and redacted) exactly as confidently as one that passes. These
//! validators exist to close that gap for the categories that have real checksums.

use sha2::{Digest, Sha256};

fn sha256(data: &[u8]) -> Vec<u8> {
    let mut h = Sha256::new();
    h.update(data);
    h.finalize().to_vec()
}

/// Luhn (mod-10) check over the digit characters of `s`.
pub fn luhn_valid(s: &str) -> bool {
    let digits: Vec<u32> = s.chars().filter_map(|c| c.to_digit(10)).collect();
    if digits.len() < 2 {
        return false;
    }
    let parity = digits.len() % 2;
    let mut sum = 0u32;
    for (i, &d) in digits.iter().enumerate() {
        let mut v = d;
        if i % 2 == parity {
            v *= 2;
            if v > 9 {
                v -= 9;
            }
        }
        sum += v;
    }
    sum.is_multiple_of(10)
}

/// Credit card: strip separators, require 12-19 digits, then Luhn.
pub fn validate_credit_card(text: &str) -> Option<bool> {
    let sanitized: String = text.chars().filter(|c| c.is_ascii_digit()).collect();
    if sanitized.len() < 12 || sanitized.len() > 19 {
        return Some(false);
    }
    Some(luhn_valid(&sanitized))
}

/// IBAN: ISO 13616 mod-97 check (remainder must be 1).
pub fn validate_iban(text: &str) -> Option<bool> {
    let s: String = text
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .map(|c| c.to_ascii_uppercase())
        .collect();
    if s.len() < 15 || s.len() > 34 {
        return Some(false);
    }
    let (first4, rest) = s.split_at(4);
    let rearranged: String = format!("{rest}{first4}");

    // Compute the big number modulo 97 without bignum arithmetic.
    let mut remainder: u32 = 0;
    for ch in rearranged.chars() {
        if let Some(d) = ch.to_digit(10) {
            remainder = (remainder * 10 + d) % 97;
        } else if ch.is_ascii_uppercase() {
            // A=10 ... Z=35 -- two decimal digits.
            let v = ch as u32 - 'A' as u32 + 10;
            remainder = (remainder * 100 + v) % 97;
        } else {
            return Some(false);
        }
    }
    Some(remainder == 1)
}

/// US SSN: reject structurally-impossible numbers (area 000/666/9xx, group 00, serial
/// 0000). Returns `None` for plausible numbers so the pattern's medium confidence score
/// is preserved rather than promoted to 1.0 -- unlike the other validators here, an SSN
/// has no checksum that can positively confirm it, only ways to rule one out.
pub fn validate_us_ssn(text: &str) -> Option<bool> {
    let d: String = text.chars().filter(|c| c.is_ascii_digit()).collect();
    if d.len() != 9 {
        return Some(false);
    }
    let area = &d[0..3];
    let group = &d[3..5];
    let serial = &d[5..9];
    if area == "000" || area == "666" || area.starts_with('9') || group == "00" || serial == "0000"
    {
        return Some(false);
    }
    None
}

/// VIN transliteration: digits keep their value; letters map A-Z (I, O, Q excluded) to
/// 1-9 per ISO 3779 / NHTSA. Returns `None` for illegal characters.
fn vin_translit(c: char) -> Option<u32> {
    if let Some(d) = c.to_digit(10) {
        return Some(d);
    }
    Some(match c {
        'A' | 'J' => 1,
        'B' | 'K' | 'S' => 2,
        'C' | 'L' | 'T' => 3,
        'D' | 'M' | 'U' => 4,
        'E' | 'N' | 'V' => 5,
        'F' | 'W' => 6,
        'G' | 'P' | 'X' => 7,
        'H' | 'Y' => 8,
        'R' | 'Z' => 9,
        _ => return None,
    })
}

/// VIN: 17 chars, ISO 3779 mod-11 check digit at position 9 (`X` == 10).
/// North-American VINs (WMI 1-5) with a bad check digit are rejected (`Some(false)`);
/// elsewhere a bad check digit yields `None` (many real non-NA VINs omit a valid check
/// digit), preserving the base score.
pub fn validate_vin(text: &str) -> Option<bool> {
    let s: String = text
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .map(|c| c.to_ascii_uppercase())
        .collect();
    if s.len() != 17 {
        return Some(false);
    }
    const WEIGHTS: [u32; 17] = [8, 7, 6, 5, 4, 3, 2, 10, 0, 9, 8, 7, 6, 5, 4, 3, 2];
    let chars: Vec<char> = s.chars().collect();
    let mut sum = 0u32;
    for (i, &c) in chars.iter().enumerate() {
        let Some(v) = vin_translit(c) else {
            return Some(false);
        };
        sum += v * WEIGHTS[i];
    }
    let remainder = sum % 11;
    let expected = if remainder == 10 {
        'X'
    } else {
        char::from_digit(remainder, 10).unwrap()
    };
    let north_american = matches!(chars[0], '1'..='5');
    if chars[8] == expected {
        Some(true)
    } else if north_american {
        Some(false)
    } else {
        None
    }
}

const B58: &[u8; 58] = b"123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz";

/// Decode a Base58 (Bitcoin alphabet) string into bytes. Returns `None` on any character
/// outside the alphabet.
fn base58_decode(s: &str) -> Option<Vec<u8>> {
    let mut bytes: Vec<u8> = Vec::with_capacity(s.len());
    for ch in s.bytes() {
        let value = B58.iter().position(|&b| b == ch)? as u32;
        let mut carry = value;
        for byte in bytes.iter_mut() {
            carry += (*byte as u32) * 58;
            *byte = (carry & 0xff) as u8;
            carry >>= 8;
        }
        while carry > 0 {
            bytes.push((carry & 0xff) as u8);
            carry >>= 8;
        }
    }
    // Leading '1's in Base58 encode leading zero bytes.
    for ch in s.bytes() {
        if ch == b'1' {
            bytes.push(0);
        } else {
            break;
        }
    }
    bytes.reverse();
    Some(bytes)
}

/// Bitcoin/Litecoin legacy address: Base58Check decode and verify the 4-byte
/// double-SHA256 checksum. Both chains share the same Base58Check construction, so this
/// validates `1`/`3` (BTC P2PKH/P2SH) and `L`/`M` (LTC) addresses -- the shapes
/// [`super::patterns::builtin_patterns`]'s `CryptoWallet` pattern matches.
pub fn validate_crypto_wallet(text: &str) -> Option<bool> {
    let Some(data) = base58_decode(text) else {
        return Some(false);
    };
    if data.len() < 5 {
        return Some(false);
    }
    let (payload, checksum) = data.split_at(data.len() - 4);
    let hash = sha256(&sha256(payload));
    Some(&hash[0..4] == checksum)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn should_validate_a_correct_luhn_number() {
        assert!(luhn_valid("4111111111111111"));
    }

    #[test]
    fn should_reject_an_incorrect_luhn_number() {
        assert!(!luhn_valid("4111111111111112"));
    }

    #[test]
    fn should_validate_a_real_visa_test_card() {
        assert_eq!(validate_credit_card("4111-1111-1111-1111"), Some(true));
    }

    #[test]
    fn should_reject_a_luhn_invalid_card_number() {
        assert_eq!(validate_credit_card("1234-5678-9012-3456"), Some(false));
    }

    #[test]
    fn should_validate_a_real_iban() {
        assert_eq!(validate_iban("GB82 WEST 1234 5698 7654 32"), Some(true));
    }

    #[test]
    fn should_reject_an_iban_with_a_bad_check_digits() {
        assert_eq!(validate_iban("GB00 WEST 1234 5698 7654 32"), Some(false));
    }

    #[test]
    fn should_reject_an_ssn_with_area_000() {
        assert_eq!(validate_us_ssn("000-12-3456"), Some(false));
    }

    #[test]
    fn should_reject_an_ssn_with_area_666() {
        assert_eq!(validate_us_ssn("666-12-3456"), Some(false));
    }

    #[test]
    fn should_reject_an_ssn_with_a_9_prefixed_area() {
        assert_eq!(validate_us_ssn("912-12-3456"), Some(false));
    }

    #[test]
    fn should_reject_an_ssn_with_group_00() {
        assert_eq!(validate_us_ssn("123-00-4567"), Some(false));
    }

    #[test]
    fn should_reject_an_ssn_with_serial_0000() {
        assert_eq!(validate_us_ssn("123-45-0000"), Some(false));
    }

    #[test]
    fn should_have_no_opinion_on_a_structurally_plausible_ssn() {
        assert_eq!(validate_us_ssn("123-45-6789"), None);
    }

    #[test]
    fn should_validate_a_correct_vin() {
        // All-ones VIN: mod-11 check digit resolves to '1' -- valid.
        assert_eq!(validate_vin("11111111111111111"), Some(true));
        assert_eq!(validate_vin("1M8GDM9AXKP042788"), Some(true));
    }

    #[test]
    fn should_reject_a_north_american_vin_with_a_wrong_check_digit() {
        // North-American (WMI '1') with a wrong check digit -> rejected.
        assert_eq!(validate_vin("12111111111111111"), Some(false));
    }

    #[test]
    fn should_reject_a_vin_with_an_illegal_character_or_wrong_length() {
        assert_eq!(validate_vin("1I111111111111111"), Some(false));
        assert_eq!(validate_vin("ABC"), Some(false));
    }

    #[test]
    fn should_validate_a_real_bitcoin_address() {
        assert_eq!(
            validate_crypto_wallet("1BvBMSEYstWetqTFn5Au4m4GFg7xJaNVN2"),
            Some(true)
        );
    }

    #[test]
    fn should_reject_a_bitcoin_address_with_a_bad_checksum() {
        assert_eq!(
            validate_crypto_wallet("1BvBMSEYstWetqTFn5Au4m4GFg7xJaNVN3"),
            Some(false)
        );
    }
}
