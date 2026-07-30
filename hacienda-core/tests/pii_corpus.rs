//! Track C2/L6's shared fixture corpus, asserted from the Rust side.
//!
//! `fixtures/pii-corpus.json` (repo root) is the other half of "one corpus, two
//! runners, identical output" — `apps/hacienda-studio` asserts the same file from
//! `vitest` against the wasm32 build. Category names in the fixture are
//! `PiiCategory`'s serde form (snake_case), the same shape that crosses the
//! wasm-bindgen boundary, so there is no separate translation layer between the two
//! runners to drift out of sync.

use hacienda_core::pii::{PiiCategory, RegexEngine};
use serde::Deserialize;
use std::collections::HashSet;

#[derive(Deserialize)]
struct Corpus {
    cases: Vec<Case>,
}

#[derive(Deserialize)]
struct Case {
    id: String,
    text: String,
    categories: Vec<PiiCategory>,
}

fn corpus() -> Corpus {
    let raw = include_str!("../../fixtures/pii-corpus.json");
    serde_json::from_str(raw).expect("fixtures/pii-corpus.json must be valid JSON")
}

#[test]
fn should_detect_exactly_the_expected_category_per_corpus_case() {
    let engine = RegexEngine::new().expect("built-in patterns compile");

    for case in corpus().cases {
        let expected: HashSet<PiiCategory> = case.categories.into_iter().collect();
        let actual: HashSet<PiiCategory> = engine
            .find_all(&case.text)
            .into_iter()
            .map(|entity| entity.category)
            .collect();

        assert_eq!(
            actual, expected,
            "case '{}' (text: {:?}): expected categories {:?}, got {:?}",
            case.id, case.text, expected, actual
        );
    }
}
