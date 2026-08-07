//! `hacienda pii reveal`: reverses a pseudonym token minted by `extract --mode
//! pseudonymize`, and collapses every reveal-time error to one generic message so a
//! caller cannot use CLI error text to probe which part of a token or key is wrong —
//! mirroring `hacienda-api/src/error.rs`'s `PseudonymError` collapsing.
//!
//! `HACIENDA_PSEUDONYM_ACTIVE_KEY`/`HACIENDA_PSEUDONYM_KEY_<ID>` are process-global;
//! every test sets or clears them explicitly rather than relying on ambient state, per
//! the discipline `extract.rs`'s `run_without_pseudonym_key` already establishes.

use std::io::Write;
use std::process::Command;
use tempfile::NamedTempFile;

const KNOWN_EMAIL: &str = "jane.doe@example.com";
const KEY_ID: &str = "k1";
// 64 bytes of hex (`KEY_BYTES` in `hacienda_core::redaction::pseudonym`), matching the
// length that crate's own tests use — any valid hex of the right length works.
const KEY_BYTES: usize = 64;
const KEY_HEX: &str = "aa";

fn key_hex() -> String {
    KEY_HEX.repeat(KEY_BYTES)
}

fn fixture() -> NamedTempFile {
    let mut file = NamedTempFile::with_suffix(".txt").expect("create fixture file");
    writeln!(
        file,
        "Please route correspondence to {KNOWN_EMAIL} rather than the front desk."
    )
    .expect("write fixture contents");
    file
}

fn run_with_key(args: &[&str]) -> (bool, String) {
    let output = Command::new(env!("CARGO_BIN_EXE_hacienda"))
        .args(args)
        .env("HACIENDA_PSEUDONYM_ACTIVE_KEY", KEY_ID)
        .env(
            format!("HACIENDA_PSEUDONYM_KEY_{}", KEY_ID.to_uppercase()),
            key_hex(),
        )
        .output()
        .expect("failed to run the hacienda binary");
    let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
    text.push_str(&String::from_utf8_lossy(&output.stderr));
    (output.status.success(), text)
}

fn run_without_key(args: &[&str]) -> (bool, String) {
    let output = Command::new(env!("CARGO_BIN_EXE_hacienda"))
        .args(args)
        .env_remove("HACIENDA_PSEUDONYM_ACTIVE_KEY")
        .output()
        .expect("failed to run the hacienda binary");
    let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
    text.push_str(&String::from_utf8_lossy(&output.stderr));
    (output.status.success(), text)
}

/// Extracts the first `[EMAIL:...]` pseudonym token from `text`, panicking with the full
/// text on failure so a broken extraction is easy to diagnose from test output.
fn extract_token(text: &str) -> String {
    let start = text.find("[EMAIL:").unwrap_or_else(|| {
        panic!("no [EMAIL:...] pseudonym token found in extract output:\n{text}")
    });
    let end = text[start..]
        .find(']')
        .map(|i| start + i + 1)
        .unwrap_or_else(|| panic!("unterminated pseudonym token in extract output:\n{text}"));
    text[start..end].to_string()
}

#[test]
fn should_reveal_a_token_minted_by_pseudonymize_mode() {
    let fixture = fixture();
    let path = fixture.path().to_str().expect("utf-8 fixture path");

    let (ok, extract_text) = run_with_key(&[
        "extract",
        "--mode",
        "pseudonymize",
        "--format",
        "json",
        path,
    ]);
    assert!(
        ok,
        "`extract --mode pseudonymize` with a key configured did not exit 0:\n{extract_text}"
    );
    let token = extract_token(&extract_text);

    let (ok, reveal_text) = run_with_key(&["pii", "reveal", &token]);
    assert!(ok, "`pii reveal` did not exit 0:\n{reveal_text}");
    assert!(
        reveal_text.contains(KNOWN_EMAIL),
        "`pii reveal` did not recover the known email address:\n{reveal_text}"
    );
}

#[test]
fn should_support_json_format() {
    let fixture = fixture();
    let path = fixture.path().to_str().expect("utf-8 fixture path");

    let (_, extract_text) = run_with_key(&[
        "extract",
        "--mode",
        "pseudonymize",
        "--format",
        "json",
        path,
    ]);
    let token = extract_token(&extract_text);

    let (ok, reveal_text) = run_with_key(&["pii", "reveal", "--format", "json", &token]);
    assert!(
        ok,
        "`pii reveal --format json` did not exit 0:\n{reveal_text}"
    );

    let value: serde_json::Value = serde_json::from_str(&reveal_text).unwrap_or_else(|e| {
        panic!("`pii reveal --format json` did not print valid JSON: {e}\n{reveal_text}")
    });
    assert_eq!(
        value["plaintext"].as_str(),
        Some(KNOWN_EMAIL),
        "json output does not carry the revealed plaintext:\n{reveal_text}"
    );
    assert!(
        value.get("audit_chain_tip").is_some(),
        "json output does not carry audit_chain_tip:\n{reveal_text}"
    );
}

/// A malformed token and a token minted under a key this process cannot resolve must fail
/// with the *same* generic message — neither the plaintext nor any variant-specific detail
/// (which part of the token/key was wrong) may leak into the difference between them.
#[test]
fn should_collapse_every_pseudonym_error_to_one_generic_message() {
    let (malformed_ok, malformed_text) = run_with_key(&["pii", "reveal", "not-a-token"]);
    let (unknown_key_ok, unknown_key_text) =
        run_with_key(&["pii", "reveal", "[EMAIL:unknownkey:AAAAAAAA]"]);

    assert!(
        !malformed_ok,
        "a malformed token exited 0:\n{malformed_text}"
    );
    assert!(
        !unknown_key_ok,
        "a token minted under an unresolvable key exited 0:\n{unknown_key_text}"
    );
    assert_eq!(
        malformed_text.trim(),
        unknown_key_text.trim(),
        "a malformed token and an unknown-key token must fail identically, not with \
         variant-specific detail an attacker could use to probe a token"
    );
    for text in [&malformed_text, &unknown_key_text] {
        assert!(
            !text.contains(KNOWN_EMAIL),
            "a refused reveal leaked the known email address:\n{text}"
        );
    }
}

/// Facade-construction failure (no active key at all) is an operator misconfiguration,
/// not an attacker probing a token, and is deliberately *not* collapsed — it should say
/// what's missing.
#[test]
fn should_report_a_specific_error_when_no_key_is_configured_at_all() {
    let (ok, text) = run_without_key(&["pii", "reveal", "[EMAIL:k1:AAAAAAAA]"]);
    assert!(!ok, "`pii reveal` with no key configured exited 0:\n{text}");
    assert!(
        text.to_lowercase().contains("pseudonym"),
        "the construction-time failure does not mention the missing pseudonym key:\n{text}"
    );
}
