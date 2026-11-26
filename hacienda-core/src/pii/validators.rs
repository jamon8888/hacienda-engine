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

/// True when the digits open with an issuer prefix (IIN) a real network hands
/// out, at a length that network actually issues. Luhn is a transcription
/// check, not an identity check: roughly one in ten arbitrary digit strings
/// of the right length passes it, and every French SIRET is Luhn-valid by the
/// same rule. Requiring an allocated IIN at a valid length is what keeps an
/// invoice SIRET from being redacted (and reported) as a credit card.
fn has_issuer_prefix(digits: &[u32]) -> bool {
    let len = digits.len();
    // Leading `n` digits as a number, for range comparisons.
    let lead = |n: usize| -> u32 { digits.iter().take(n).fold(0, |acc, d| acc * 10 + d) };

    // Visa
    if lead(1) == 4 {
        return matches!(len, 13 | 16 | 19);
    }
    // Mastercard: the classic 51-55 block and the newer 2221-2720 one.
    if (51..=55).contains(&lead(2)) || (2221..=2720).contains(&lead(4)) {
        return len == 16;
    }
    // American Express
    if matches!(lead(2), 34 | 37) {
        return len == 15;
    }
    // Discover
    if lead(4) == 6011
        || (622126..=622925).contains(&lead(6))
        || (644..=649).contains(&lead(3))
        || lead(2) == 65
    {
        return matches!(len, 16 | 19);
    }
    // Diners Club — the one that genuinely overlaps SIRET at 14 digits. Kept
    // anyway: dropping a real card format to dodge a false positive is the
    // wrong trade for a redaction tool.
    if (300..=305).contains(&lead(3)) || lead(4) == 3095 || matches!(lead(2), 36 | 38 | 39) {
        return len == 14;
    }
    // JCB
    if (3528..=3589).contains(&lead(4)) {
        return (16..=19).contains(&len);
    }
    // UnionPay
    if lead(2) == 62 {
        return (16..=19).contains(&len);
    }
    false
}

/// Credit card: strip separators, require 13-19 digits, a real issuer prefix
/// at a length that network issues, then Luhn.
pub fn validate_credit_card(text: &str) -> Option<bool> {
    let digits: Vec<u32> = text.chars().filter_map(|c| c.to_digit(10)).collect();
    if !(13..=19).contains(&digits.len()) || !has_issuer_prefix(&digits) {
        return Some(false);
    }
    Some(luhn_valid_digits(&digits))
}

/// Luhn (mod-10) over an existing digit slice, used by [`validate_credit_card`]
/// after sanitisation so the digits are not collected twice.
fn luhn_valid_digits(digits: &[u32]) -> bool {
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

// ---- EU VAT ---------------------------------------------------------------
// Ported from redact-paperasse's `eu_vat.rs` recognizer. Member states do not
// offer the same guarantees: FR/BE/NL/LU have verifiable check digits, the
// rest only have a stable country-code + body shape. Claiming 1.0 for the
// latter would assert a check that never ran, and a *failed* checksum is
// stronger evidence than an unrun one, so it is rejected outright.

fn digits_as_u64(s: &str) -> Option<u64> {
    if s.is_empty() || !s.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    s.parse::<u64>().ok()
}

fn all_digits(s: &str, n: usize) -> bool {
    s.len() == n && s.bytes().all(|b| b.is_ascii_digit())
}

fn digits_in(s: &str, range: std::ops::RangeInclusive<usize>) -> bool {
    range.contains(&s.len()) && s.bytes().all(|b| b.is_ascii_digit())
}

/// `Some(1.0)` when check digits were verified, `Some(0.8)` when only the
/// country code and body shape were checked, `None` when rejected.
fn validate_eu_vat_parts(country: &str, body: &str) -> Option<f32> {
    match country {
        // ---- checksummed ----
        "FR" => {
            // 2-character key + 9-digit SIREN. Old-style numeric keys satisfy
            // key == (12 + 3 * (SIREN mod 97)) mod 97; newer ones contain
            // letters and carry no verifiable check, so they fall back to
            // format-only rather than being dropped.
            if body.len() != 11 {
                return None;
            }
            let (key, siren) = body.split_at(2);
            if !all_digits(siren, 9) || !key.bytes().all(|b| b.is_ascii_alphanumeric()) {
                return None;
            }
            match (digits_as_u64(key), digits_as_u64(siren)) {
                (Some(k), Some(s)) if (12 + 3 * (s % 97)) % 97 == k => Some(1.0),
                // Letter in the key: shape is right, check is undefined.
                (None, Some(_)) => Some(0.8),
                _ => None,
            }
        }
        "BE" => {
            // 10 digits; the last two are 97 minus the first eight mod 97.
            let n = digits_as_u64(body).filter(|_| body.len() == 10)?;
            if (97 - (n / 100) % 97) == n % 100 {
                Some(1.0)
            } else {
                None
            }
        }
        "NL" => {
            // 9 digits, then 'B', then 2 digits. The 9 digits carry a
            // weighted mod-11 check.
            if body.len() != 12 || !body[9..10].eq_ignore_ascii_case("B") {
                return None;
            }
            let head = &body[..9];
            if !all_digits(head, 9) || !all_digits(&body[10..], 2) {
                return None;
            }
            let d: Vec<u64> = head.bytes().map(|b| u64::from(b - b'0')).collect();
            let sum: u64 = d[..8].iter().zip((2..=9).rev()).map(|(v, w)| v * w).sum();
            if sum % 11 == d[8] {
                Some(1.0)
            } else {
                None
            }
        }
        "LU" => {
            // 8 digits; first six mod 89 give the last two.
            let n = digits_as_u64(body).filter(|_| body.len() == 8)?;
            if (n / 100) % 89 == n % 100 {
                Some(1.0)
            } else {
                None
            }
        }

        // ---- format-checked only (0.8) ----
        "AT" => {
            (body.len() == 9 && body[..1].eq_ignore_ascii_case("U") && all_digits(&body[1..], 8))
                .then_some(0.8)
        }
        "DE" | "EE" | "EL" | "PT" => all_digits(body, 9).then_some(0.8),
        "DK" | "FI" | "HU" | "MT" | "SI" => all_digits(body, 8).then_some(0.8),
        "BG" => digits_in(body, 9..=10).then_some(0.8),
        "CZ" => digits_in(body, 8..=10).then_some(0.8),
        "PL" | "SK" => all_digits(body, 10).then_some(0.8),
        "HR" | "IT" | "LV" => all_digits(body, 11).then_some(0.8),
        "SE" => all_digits(body, 12).then_some(0.8),
        "LT" => (all_digits(body, 9) || all_digits(body, 12)).then_some(0.8),
        "RO" => digits_in(body, 2..=10).then_some(0.8),
        "CY" => (body.len() == 9
            && all_digits(&body[..8], 8)
            && body.as_bytes()[8].is_ascii_alphabetic())
        .then_some(0.8),
        "ES" => (body.len() == 9
            && body.as_bytes()[0].is_ascii_alphanumeric()
            && all_digits(&body[1..8], 7)
            && body.as_bytes()[8].is_ascii_alphanumeric())
        .then_some(0.8),
        "IE" => {
            let b = body.as_bytes();
            let modern = (8..=9).contains(&body.len())
                && all_digits(&body[..7], 7)
                && b[7..].iter().all(|c| c.is_ascii_alphabetic());
            let legacy = body.len() == 8
                && b[0].is_ascii_digit()
                && b[1].is_ascii_alphanumeric()
                && all_digits(&body[2..7], 5)
                && b[7].is_ascii_alphabetic();
            (modern || legacy).then_some(0.8)
        }
        _ => None,
    }
}

/// EU VAT adapter for [`PatternMeta::validator`](super::types::PatternMeta::validator):
/// parses `CC body` out of the matched span and maps the three-way result onto
/// `Option<bool>`. `Some(true)` (checksum verified) promotes confidence to 1.0;
/// `None` (format-only) keeps the pattern's 0.8 base; `Some(false)` rejects.
pub fn validate_eu_vat(text: &str) -> Option<bool> {
    // Non-breaking spaces are how invoices typeset "FR 40303265045".
    let normalized: String = text
        .chars()
        .map(|c| if c == '\u{a0}' { ' ' } else { c })
        .collect();
    let s = normalized.trim().to_ascii_uppercase();
    if s.len() < 4 || !s.is_ascii() {
        return Some(false);
    }
    let country = &s[..2];
    if !country.bytes().all(|b| b.is_ascii_uppercase()) {
        return Some(false);
    }
    let body = s[2..].trim_start();
    match validate_eu_vat_parts(country, body) {
        Some(1.0) => Some(true),
        Some(_) => None,
        None => Some(false),
    }
}

/// French NIR (numéro de sécurité sociale / carte vitale): INSEE mod-97 check.
/// Format is `S YY MM DD CCC OOO KK` — 13 significant digits plus a 2-digit
/// key, written with or without spaces/dots. Corsican department codes `2A`
/// and `2B` are substituted to `19`/`18` before the check.
pub fn validate_fr_nir(text: &str) -> Option<bool> {
    let cleaned: String = text
        .chars()
        .filter(|c| !matches!(c, ' ' | '.' | '-'))
        .flat_map(char::to_uppercase)
        .collect();
    if cleaned.chars().count() != 15 {
        return Some(false);
    }
    let digits_part: String = cleaned.chars().take(13).collect();
    let key_part: String = cleaned.chars().skip(13).collect();
    let checksum_input = digits_part.replace("2A", "19").replace("2B", "18");
    let Ok(n) = checksum_input.parse::<u64>() else {
        return Some(false);
    };
    let Ok(key) = key_part.parse::<u64>() else {
        return Some(false);
    };
    Some(97 - (n % 97) == key)
}

// ---- French SIREN / SIRET --------------------------------------------------
// Both are Luhn-validated, but Luhn passes roughly one arbitrary string in
// ten, so an unanchored match would redact invoice totals and order references
// on every French document. The anchor is what makes these categories safe.

/// La Poste is the documented exception, but narrower than usually described:
/// its SIREN does satisfy Luhn; it is the *establishments* that do not — La
/// Poste SIRETs validate by digit-sum divisible by 5 instead.
const LA_POSTE_SIREN: &str = "356000000";

/// True when one of the labels appears shortly before `start`. Case-insensitive
/// and tolerant of the `N°`/`:`/newline noise that sits between a label and its
/// value on a real form. `EUID` counts because the European identifier embeds
/// the SIREN verbatim (`FR7501.944681634`).
fn labelled_before(text: &str, start: u32) -> bool {
    let end = start as usize;
    if end > text.len() || !text.is_char_boundary(end) {
        return false;
    }
    let from = text[..end]
        .char_indices()
        .rev()
        .nth(48)
        .map_or(0, |(i, _)| i);
    let window = text[from..end].to_ascii_uppercase();
    ["SIRET", "SIREN", "RCS", "EUID"]
        .iter()
        .any(|key| window.contains(key))
}

/// The canonical French SIRET display grouping: 3-3-3-5 with a space or dot
/// between every group. French typesets nothing else that way (thousands
/// grouping on 14 digits would be 2-3-3-3-3), so the grouping alone is an
/// anchor. Partial or absent separators still require a label.
fn has_siret_grouping(s: &str) -> bool {
    let b = s.as_bytes();
    if b.len() != 17 {
        return false;
    }
    b.iter().enumerate().all(|(i, &c)| {
        if matches!(i, 3 | 7 | 11) {
            matches!(c, b' ' | b'.')
        } else {
            c.is_ascii_digit()
        }
    })
}

/// The French 3-3-3 display grouping of a nine-digit SIREN. Used by the phone
/// validator to refuse such a span: it is the first nine digits of a grouped
/// SIRET (`732 829 320 00074`) and French phones are written in five pairs,
/// never 3-3-3.
fn has_siren_grouping(s: &str) -> bool {
    let b = s.as_bytes();
    if b.len() != 11 {
        return false;
    }
    b.iter().enumerate().all(|(i, &c)| {
        if matches!(i, 3 | 7) {
            matches!(c, b' ' | b'.')
        } else {
            c.is_ascii_digit()
        }
    })
}

/// Luhn + structural check for a SIREN (9 digits). The label anchor is applied
/// separately via [`anchor_fr_siren`].
pub fn validate_fr_siren(text: &str) -> Option<bool> {
    let digits: String = text.chars().filter(|c| c.is_ascii_digit()).collect();
    if digits.len() != 9 {
        return Some(false);
    }
    Some(luhn_valid(&digits))
}

/// SIREN must carry a nearby label. Grouping alone is deliberately *not*
/// accepted: `732 829 320` passes Luhn and is exactly how French typesets
/// millions, so an unlabelled grouped match would redact money.
pub fn anchor_fr_siren(text: &str, start: u32, matched: &str) -> bool {
    if !labelled_before(text, start) {
        return false;
    }
    // Un SIRET groupé commence par un SIREN valide : « 732 829 320 00074 ».
    // Sans cette garde, le motif SIREN (plus court) gagne le dédup par
    // chevauchement et le SIRET disparaît au profit de son propre préfixe.
    let end = start as usize + matched.len();
    if end <= text.len() {
        let reste = text[end..].trim_start_matches([' ', '.']).as_bytes();
        if reste.len() >= 5
            && reste[..5].iter().all(u8::is_ascii_digit)
            && (reste.len() == 5 || !reste[5].is_ascii_digit())
        {
            return false;
        }
    }
    true
}

/// Luhn + structural check for a SIRET (14 digits), including the La Poste
/// digit-sum carve-out. The label/grouping anchor is applied separately via
/// [`anchor_fr_siret`].
pub fn validate_fr_siret(text: &str) -> Option<bool> {
    let digits: String = text.chars().filter(|c| c.is_ascii_digit()).collect();
    if digits.len() != 14 {
        return Some(false);
    }
    let valid = if digits.starts_with(LA_POSTE_SIREN) {
        digits
            .chars()
            .filter_map(|c| c.to_digit(10))
            .sum::<u32>()
            .is_multiple_of(5)
    } else {
        luhn_valid(&digits)
    };
    Some(valid)
}

/// A SIRET is anchored either by a nearby label or by the canonical 3-3-3-5
/// display grouping. The grouping matters on forms: a real CFE declaration
/// carried its SIRET in an address block with the nearest label 2,758
/// characters away on a later page, so any label-window rule fails there.
pub fn anchor_fr_siret(text: &str, start: u32, matched: &str) -> bool {
    labelled_before(text, start) || has_siret_grouping(matched)
}

/// Une IPv4 ne doit pas être capturée au milieu d'une suite pointée plus
/// longue — un NIR français pointé (`2.96.01.12.345.678.59`) commence par
/// quatre groupes qui ont la forme d'une adresse IP.
pub fn anchor_ipv4(text: &str, start: u32, matched: &str) -> bool {
    let bytes = text.as_bytes();
    let debut = start as usize;
    let fin = debut + matched.len();
    if fin < bytes.len() && bytes[fin] == b'.' && bytes.get(fin + 1).is_some_and(u8::is_ascii_digit)
    {
        return false;
    }
    if debut >= 2 && bytes[debut - 1] == b'.' && bytes[debut - 2].is_ascii_digit() {
        return false;
    }
    true
}

/// Phone number: validate a regex-found candidate with Google libphonenumber
/// metadata (via `rlibphonenumber`). Regexes alone cannot know that
/// `01 56 35 91 80` is a French national number while `732 829 320 00074` is
/// a SIRET — the matcher knows, including OCR-merged groupings such as
/// `0156 35 9180`.
///
/// The matcher is deliberately used only as a *validator* on the candidate
/// span, not as the span producer: run over a whole document it can pick the
/// trailing `00074` of a SIRET out as a phone, which would be a false positive
/// our regex-derived spans avoid.
///
/// Returns `None` (no opinion) for a real phone so the pattern's 0.75 base
/// confidence is kept — unlike a checksum validator, a valid phone is not
/// proof of identity. `Some(false)` rejects an invalid candidate or one whose
/// shape belongs to another recognizer (SSN, EIN, SIREN/SIRET, NIR, IPv4).
pub fn validate_phone_number(text: &str) -> Option<bool> {
    use rlibphonenumber::phonenumber_matcher::FindNumberExt;

    let trimmed = text.trim();
    // Same policy as redact-paperasse: a fully compact digit run is too
    // ambiguous with account/order numbers to claim as a phone by default,
    // even when libphonenumber accepts it (it does accept `0156359180`).
    // If OCR drops every separator we miss that number; that trade is
    // deliberate and documented here so changing it is a decision.
    if !trimmed.chars().any(|c| !c.is_ascii_digit()) {
        return Some(false);
    }
    // Une adresse IPv4 est une suite de quatre groupes de chiffres séparés
    // par des points : le motif de téléphone la capture, mais ce n'est pas
    // un numéro et libphonenumber peut l'accepter dans certaines régions.
    if trimmed.parse::<std::net::Ipv4Addr>().is_ok() {
        return Some(false);
    }

    // Formes qui appartiennent à un autre recognizer. Sans ces gardes,
    // libphonenumber valide « 123-45-6789 » comme un numéro et le motif
    // téléphone (même placé en dernier) vole l'embranchement d'un SSN
    // impossible, déjà rejeté par son propre validateur.
    let b = trimmed.as_bytes();
    let all_digits = |slice: &[u8]| slice.iter().all(u8::is_ascii_digit);
    let ssn_shape = b.len() == 11
        && b[3] == b'-'
        && b[6] == b'-'
        && all_digits(&b[..3])
        && all_digits(&b[4..6])
        && all_digits(&b[7..]);
    let ein_shape = b.len() == 10 && b[2] == b'-' && all_digits(&b[..2]) && all_digits(&b[3..]);
    let digit_count = b.iter().filter(|c| c.is_ascii_digit()).count();
    let nir_shape = digit_count == 15
        && matches!(b.first().copied(), Some(b'1' | b'2'))
        && b.iter()
            .all(|c| c.is_ascii_digit() || matches!(c, b' ' | b'.' | b'-'));
    if ssn_shape
        || ein_shape
        || nir_shape
        || has_siret_grouping(trimmed)
        || has_siren_grouping(trimmed)
    {
        return Some(false);
    }

    let covers_whole_candidate = trimmed
        .find_phone_numbers_auto_region()
        .any(|m| m.start == 0 && m.end() == trimmed.len());
    // Un numéro de téléphone n'a pas de somme de contrôle : `None` dit
    // « aucune opinion », ce qui conserve `base_confidence` (0.75) au lieu
    // de promouvoir le résultat à 1.0 comme le fait un vrai checksum.
    if covers_whole_candidate {
        None
    } else {
        Some(false)
    }
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
    // `remainder` is 0..=10: 10 is the ISO 3779 'X' case, and `char::from_digit` always
    // succeeds for 0..=9. `unwrap_or('X')` reaches its fallback only on 10, so it is
    // equivalent to the explicit branch it replaces — not a masked failure — while
    // keeping this a no-panic path per this crate's "no unwrap/expect in library code"
    // rule, in case `WEIGHTS` or the modulus ever changes shape.
    let expected = char::from_digit(remainder, 10).unwrap_or('X');
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
    fn should_reject_a_french_siret_that_passes_luhn() {
        // 14 digits, Luhn-valid, exactly like a Diners card — but no network
        // issues cards starting 73. Every French invoice used to be flagged as
        // a credit card before the issuer-prefix check existed.
        assert_eq!(validate_credit_card("73282932000074"), Some(false));
        assert_eq!(validate_credit_card("732 829 320 00074"), Some(false));
        assert_eq!(validate_credit_card("944 681 634 00019"), Some(false));
    }

    #[test]
    fn should_reject_a_luhn_valid_number_with_no_issuer_prefix() {
        // Passes Luhn; 99 is not an allocated IIN.
        assert_eq!(validate_credit_card("9900000000000002"), Some(false));
    }

    #[test]
    fn should_reject_a_valid_prefix_at_the_wrong_length() {
        // Starts with 4 and passes Luhn, but Visa does not issue 18-digit cards.
        assert_eq!(validate_credit_card("400000000000000002"), Some(false));
    }

    #[test]
    fn should_validate_one_test_number_per_network_family() {
        for n in [
            "5555555555554444", // Mastercard 51-55
            "2223003122003222", // Mastercard 2-series
            "6011111111111117", // Discover
            "3530111333300000", // JCB
            "36227206271667",   // Diners, 14 digits
            "4222222222222",    // Visa, 13 digits
        ] {
            assert_eq!(validate_credit_card(n), Some(true), "should validate: {n}");
        }
    }

    #[test]
    fn should_checksum_validate_french_vat() {
        assert_eq!(validate_eu_vat("FR40303265045"), Some(true));
        assert_eq!(validate_eu_vat("fr 40303265045"), Some(true));
        assert_eq!(validate_eu_vat("FR\u{a0}40303265045"), Some(true));
        // A letter key is well-formed but has no verifiable check -> format-only.
        assert_eq!(validate_eu_vat("FRAB303265045"), None);
        // A failed numeric check is rejected, not downgraded.
        assert_eq!(validate_eu_vat("FR41303265045"), Some(false));
    }

    #[test]
    fn should_checksum_validate_be_nl_lu_vat() {
        for n in ["BE0776091951", "NL123456782B90", "LU12345613"] {
            assert_eq!(validate_eu_vat(n), Some(true), "should validate: {n}");
        }
        for n in ["BE0776091952", "NL123456783B90", "LU12345614"] {
            assert_eq!(validate_eu_vat(n), Some(false), "should reject: {n}");
        }
    }

    #[test]
    fn should_keep_format_only_vat_at_base_confidence() {
        // `None` means "no checksum ran", so `RegexEngine` keeps 0.8.
        for n in ["DE123456789", "IT12345678901", "ATU12345678", "IE1234567FA"] {
            assert_eq!(validate_eu_vat(n), None, "format-only: {n}");
        }
    }

    #[test]
    fn should_reject_non_eu_or_wrong_length_vat() {
        for n in ["US123456789", "CH123456789", "GB123456789", "DE12345678"] {
            assert_eq!(validate_eu_vat(n), Some(false), "should reject: {n}");
        }
    }

    #[test]
    fn should_validate_and_anchor_a_french_siret() {
        let n = "73282932000074";
        assert_eq!(validate_fr_siret(n), Some(true));
        assert!(anchor_fr_siret("SIRET 73282932000074", 6, n));
        assert!(anchor_fr_siret(
            "N° SIRET : 732 829 320 00074",
            11,
            "732 829 320 00074"
        ));
        // The canonical grouping is itself an anchor, with no label nearby.
        assert!(anchor_fr_siret(
            "75008 PARIS 944 681 634 00019 66198",
            12,
            "944 681 634 00019"
        ));
        assert!(!anchor_fr_siret("Référence 73282932000074", 11, n));
        assert_eq!(validate_fr_siret("73282932000075"), Some(false));
    }

    #[test]
    fn should_validate_and_anchor_a_french_siren() {
        let n = "732829320";
        assert_eq!(validate_fr_siren(n), Some(true));
        assert!(anchor_fr_siren("SIREN 732829320", 6, n));
        assert!(anchor_fr_siren("RCS Paris 732 829 320", 10, "732 829 320"));
        // Grouping alone is French millions formatting, not a SIREN.
        assert!(!anchor_fr_siren("montant : 732 829 320", 11, "732 829 320"));
        assert_eq!(validate_fr_siren("732829321"), Some(false));
    }

    #[test]
    fn should_handle_the_la_poste_siret_exception() {
        // La Poste's SIREN passes Luhn like any other; its establishments do not.
        assert_eq!(validate_fr_siren("356000000"), Some(true));
        let la_poste_siret = "35600000009075";
        assert!(!luhn_valid(la_poste_siret));
        assert_eq!(validate_fr_siret(la_poste_siret), Some(true));
    }

    #[test]
    fn should_validate_a_french_nir() {
        for n in [
            "185017512345609",       // compact
            "1 85 01 75 123 456 09", // spaced
            "2.96.01.12.345.678.59", // dotted
            "185032A03411208",       // Corsican 2A
        ] {
            assert_eq!(validate_fr_nir(n), Some(true), "should validate: {n}");
        }
    }

    #[test]
    fn should_reject_an_invalid_french_nir() {
        for n in [
            "291059933807693",  // bad checksum
            "291139933807692",  // impossible month
            "29105993380769",   // too short
            "1850175123456090", // too long
        ] {
            assert_eq!(validate_fr_nir(n), Some(false), "should reject: {n}");
        }
    }

    #[test]
    fn should_validate_phone_numbers_across_regions() {
        // French national five-pair format, OCR-merged grouping, and `+33`.
        for n in [
            "01 56 35 91 80",
            "06 12 34 56 78",
            "0156 35 9180",
            "+33 6 12 34 56 78",
        ] {
            // `None` = no opinion: a phone has no checksum, so the caller
            // keeps its 0.75 base confidence instead of promoting to 1.0.
            assert_eq!(validate_phone_number(n), None, "should accept: {n}");
        }
        // US and UK shapes.
        for n in ["212-555-0143", "(212) 555-0143", "020 7183 8750"] {
            assert_eq!(validate_phone_number(n), None, "should accept: {n}");
        }
    }

    #[test]
    fn should_not_claim_a_siret_or_compact_digit_run_as_a_phone() {
        // libphonenumber on the whole document would pick `00074` out of a
        // SIRET; the candidate-span check rejects it because no match covers
        // the submitted span.
        assert_eq!(validate_phone_number("732 829 320 00074"), Some(false));
        assert_eq!(validate_phone_number("0156359180"), Some(false));
        assert_eq!(validate_phone_number("73282932000074"), Some(false));
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
