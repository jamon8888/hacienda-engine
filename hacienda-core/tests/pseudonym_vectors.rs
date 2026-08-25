//! Golden vectors for `Pseudonymiser::token`/`reveal`, mirrored byte-for-byte in
//! `apps/hacienda-studio/lib/pseudonymize.test.ts` (its own
//! `describe("golden vectors captured from a real Pseudonymiser run (Track F2)")` block).
//!
//! Studio reimplements this crate's AES-256-SIV pseudonymization in TypeScript/WebCrypto
//! rather than binding to this crate's wasm build for it, so nothing else catches the two
//! implementations drifting apart. These are the *same four cases* hardcoded on the TS
//! side — if this crate's key-half ordering, S2V step, category-label mapping, or
//! normalization ever changes, this test and the TS one must be updated together, or the
//! two pseudonymization outputs silently diverge (a document pseudonymized by one could
//! stop being revealable by the other). There is deliberately no shared fixture file: a
//! human keeping both lists in sync, rather than one language generating the other's
//! input, is what forces a change here to be a *reviewed* change on the TS side too.

use hacienda_core::pii::PiiCategory;
use hacienda_core::redaction::{EnvKeyResolver, Pseudonymiser};
use hacienda_core::tenancy::TenantId;

const KEY_ID: &str = "k1";

fn pseudonymiser() -> Pseudonymiser {
    // `"07"` repeated to fill `KEY_BYTES` (64) bytes — the same fixed key
    // `pseudonymize.test.ts` uses as `KEY_HEX`.
    let key_hex = "07".repeat(64);
    let resolver = EnvKeyResolver::with_lookup(move |name| match name {
        "HACIENDA_PSEUDONYM_ACTIVE_KEY" => Some(KEY_ID.to_string()),
        "HACIENDA_PSEUDONYM_KEY_K1" => Some(key_hex.clone()),
        _ => None,
    });
    Pseudonymiser::new(&resolver, &TenantId::default_tenant(), &[]).expect("build pseudonymiser")
}

/// (category, input text, expected token, expected revealed/normalized text) — identical
/// to the `cases` array in `pseudonymize.test.ts`'s golden-vector block. Keep these two
/// lists in the same order so a diff between the files stays easy to eyeball.
fn cases() -> Vec<(PiiCategory, &'static str, &'static str, &'static str)> {
    vec![
        (
            PiiCategory::Email,
            "alice@example.com",
            "[EMAIL:k1:L732D2APGTRA74WMNRAYJTGCFVK2ZOENNO45PU35DRH3AOT3EZF3ELVBFVPQGK7QQ5QXUSMGL2AVY]",
            "alice@example.com",
        ),
        (
            PiiCategory::PhoneNumber,
            "+33 6 12 34 56 78",
            "[PHONENUMBER:k1:INZOAUHJKNHRVJ5LOSMINXJ657XQDEQ6HMR67RY4FITWEXOCI42Q]",
            "+33612345678",
        ),
        (
            PiiCategory::FullName,
            "Jean Dupont",
            "[FULLNAME:k1:E3P72PJ5AXA25ZBC6TUXKFDTIZVUBR3YJNMHZKJPKSSJFKUTVUOQ]",
            "jean dupont",
        ),
        (
            PiiCategory::Iban,
            "FR7630006000011234567890189",
            "[IBAN:k1:XEPIMSRZNA73NIY7N7NYDYTQ42QXNZVCQTRZ57PKEAFVKNY54PDGEIOB2C6QDUDGDKN3ARFIOAUMC]",
            "fr7630006000011234567890189",
        ),
    ]
}

#[test]
fn mints_the_exact_token_the_ts_golden_vectors_expect() {
    let pseudonymiser = pseudonymiser();
    for (category, text, expected_token, _expected_revealed) in cases() {
        let token = pseudonymiser
            .token(&category, text)
            .unwrap_or_else(|e| panic!("token({category:?}, {text:?}) failed: {e}"));
        assert_eq!(token, expected_token, "token mismatch for {category:?} {text:?}");
    }
}

#[test]
fn reveals_its_own_token_to_the_exact_text_the_ts_golden_vectors_expect() {
    let pseudonymiser = pseudonymiser();
    for (category, text, _expected_token, expected_revealed) in cases() {
        let token = pseudonymiser.token(&category, text).expect("mint token");
        let revealed = pseudonymiser
            .reveal(&token)
            .unwrap_or_else(|e| panic!("reveal({token:?}) failed: {e}"));
        assert_eq!(revealed, expected_revealed, "reveal mismatch for {category:?} {text:?}");
    }
}
