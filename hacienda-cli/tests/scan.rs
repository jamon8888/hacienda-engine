//! Task 6 of Phase 2: `scan` detects but never emits document text.
//!
//! `scan`'s entire value proposition (§6.2, D5) is that it is the honest, no-leak
//! alternative to `extract --no-redact`: report what PII is present without ever putting
//! document content — redacted or not — on stdout. A regex-only fixture keeps this test
//! independent of the NER model (`pii.model.enabled` defaults to `false`), so it needs no
//! model directory.

use std::io::Write;
use std::process::Command;
use tempfile::NamedTempFile;

const KNOWN_EMAIL: &str = "jane.doe@example.com";

fn fixture() -> NamedTempFile {
    let mut file = NamedTempFile::with_suffix(".txt").expect("create fixture file");
    writeln!(
        file,
        "Please route correspondence to {KNOWN_EMAIL} rather than the front desk."
    )
    .expect("write fixture contents");
    file
}

fn run(args: &[&str]) -> (bool, String) {
    let output = Command::new(env!("CARGO_BIN_EXE_hacienda"))
        .args(args)
        .output()
        .expect("failed to run the hacienda binary");
    let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
    text.push_str(&String::from_utf8_lossy(&output.stderr));
    (output.status.success(), text)
}

#[test]
fn should_report_detection_without_emitting_any_document_text() {
    let fixture = fixture();
    let path = fixture.path().to_str().expect("utf-8 fixture path");

    let (ok, text) = run(&["scan", path]);

    assert!(ok, "`hacienda scan` did not exit 0:\n{text}");
    assert!(
        text.contains("Email"),
        "scan output does not report the Email detection:\n{text}"
    );
    assert!(
        !text.contains(KNOWN_EMAIL),
        "scan output leaked the known email address:\n{text}"
    );
    assert!(
        !text.contains("route correspondence"),
        "scan output leaked fixture document text:\n{text}"
    );
}

#[test]
fn should_support_json_format_without_emitting_document_text() {
    let fixture = fixture();
    let path = fixture.path().to_str().expect("utf-8 fixture path");

    let (ok, text) = run(&["scan", "--format", "json", path]);

    assert!(ok, "`hacienda scan --format json` did not exit 0:\n{text}");
    let value: serde_json::Value = serde_json::from_str(&text)
        .unwrap_or_else(|e| panic!("scan --format json did not print valid JSON: {e}\n{text}"));
    let entries = value
        .as_array()
        .expect("scan --format json prints an array");
    assert_eq!(entries.len(), 1, "expected one result per input:\n{text}");
    let categories: Vec<_> = entries[0]["pii"]["entities"]
        .as_array()
        .expect("entities array")
        .iter()
        .map(|e| e["category"].as_str().unwrap_or_default())
        .collect();
    assert!(
        categories.contains(&"email"),
        "scan --format json did not report an email detection: {categories:?}\n{text}"
    );
    assert!(
        !text.contains(KNOWN_EMAIL),
        "scan --format json leaked the known email address:\n{text}"
    );
}
