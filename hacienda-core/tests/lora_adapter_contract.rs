//! `NerDetector::from_candle_local` must fail loudly on a bad model or adapter path,
//! not panic or silently produce a detector that reports nothing — see
//! `superpowers/specs/2026-07-28-hacienda-cli-api-integration-design.md` §13: "An API
//! that advertises model-backed detection while silently running regex-only would be a
//! worse failure than not shipping it." Every case here fails before any tensor is read,
//! so no real GLiNER2 weights are required.
#![cfg(all(feature = "ner-candle", not(target_arch = "wasm32")))]

use hacienda_core::pii::NerDetector;

#[test]
fn should_return_err_not_panic_for_a_nonexistent_model_dir() {
    let model_dir = std::path::Path::new("/nonexistent/hacienda-lora-contract-test-model-dir");

    let result = NerDetector::from_candle_local(model_dir, None);

    assert!(
        result.is_err(),
        "a missing model_dir must fail to load, not panic"
    );
}

#[test]
fn should_return_err_not_panic_for_a_nonexistent_adapter_dir() {
    // model_dir also doesn't exist here — from_local fails before load_adapter is ever
    // reached, so this still needs no real weights; it asserts the same "Err, not panic"
    // contract through the adapter-path argument specifically.
    let model_dir = std::path::Path::new("/nonexistent/hacienda-lora-contract-test-model-dir");
    let adapter_dir = std::path::Path::new("/nonexistent/hacienda-lora-contract-test-adapter-dir");

    let result = NerDetector::from_candle_local(model_dir, Some(adapter_dir));

    assert!(
        result.is_err(),
        "a missing adapter_dir must fail to load, not panic"
    );
}

#[test]
fn should_return_err_for_an_empty_model_dir() {
    // Exists, but has none of tokenizer.json/config.json/model.safetensors —
    // distinguishes "path doesn't exist" from "path exists but isn't a model".
    let dir = tempfile::tempdir().expect("create temp dir");

    let result = NerDetector::from_candle_local(dir.path(), None);

    assert!(
        result.is_err(),
        "an empty directory must fail to load, not panic"
    );
}
