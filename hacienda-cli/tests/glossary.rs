//! `--glossary-out <dir>` on `extract`/`scan`: writes this run's entity glossary to
//! `<dir>/glossary.json`. Glossary state lives only inside a live run's facade — there is
//! no standalone `hacienda glossary` command, see `ExtractArgs::glossary_out`.

use std::io::Write;
use std::process::Command;
use tempfile::{NamedTempFile, TempDir};

const KNOWN_EMAIL: &str = "jane.doe@example.com";

/// Mentions the same email twice so the default `min_count = 2` threshold is cleared —
/// a glossary of one-off mentions is deliberately suppressed (`GlossaryConfig::min_count`
/// doc comment), so a fixture with a single mention would produce an empty glossary and
/// prove nothing.
fn fixture() -> NamedTempFile {
    let mut file = NamedTempFile::with_suffix(".txt").expect("create fixture file");
    writeln!(
        file,
        "Please route correspondence to {KNOWN_EMAIL} rather than the front desk.\n\
         Follow-up questions also go to {KNOWN_EMAIL}."
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

fn read_glossary(dir: &TempDir) -> serde_json::Value {
    let path = dir.path().join("glossary.json");
    let bytes = std::fs::read(&path)
        .unwrap_or_else(|e| panic!("glossary.json was not written at {path:?}: {e}"));
    serde_json::from_slice(&bytes)
        .unwrap_or_else(|e| panic!("glossary.json is not valid JSON: {e}"))
}

#[test]
fn should_write_a_glossary_with_repeated_mentions_via_extract() {
    let fixture = fixture();
    let path = fixture.path().to_str().expect("utf-8 fixture path");
    let glossary_dir = TempDir::new().expect("create glossary-out directory");
    let glossary_dir_path = glossary_dir.path().to_str().expect("utf-8 dir path");

    let (ok, text) = run(&[
        "extract",
        "--mode",
        "mask",
        "--glossary-out",
        glossary_dir_path,
        path,
    ]);
    assert!(ok, "`extract --glossary-out` did not exit 0:\n{text}");

    let glossary = read_glossary(&glossary_dir);
    let entries = glossary.as_array().expect("glossary.json is a JSON array");
    assert!(
        entries
            .iter()
            .any(|e| e["category"] == "Email" && e["count"].as_u64() == Some(2)),
        "glossary does not contain an Email entry with count 2: {glossary}"
    );
}

#[test]
fn should_write_a_glossary_via_scan_without_leaking_document_text() {
    let fixture = fixture();
    let path = fixture.path().to_str().expect("utf-8 fixture path");
    let glossary_dir = TempDir::new().expect("create glossary-out directory");
    let glossary_dir_path = glossary_dir.path().to_str().expect("utf-8 dir path");

    let (ok, text) = run(&["scan", "--glossary-out", glossary_dir_path, path]);
    assert!(ok, "`scan --glossary-out` did not exit 0:\n{text}");

    let glossary = read_glossary(&glossary_dir);
    let entries = glossary.as_array().expect("glossary.json is a JSON array");
    assert!(
        entries
            .iter()
            .any(|e| e["category"] == "Email" && e["count"].as_u64() == Some(2)),
        "glossary does not contain an Email entry with count 2: {glossary}"
    );

    let raw = glossary.to_string();
    assert!(
        !raw.contains("route correspondence"),
        "glossary.json leaked fixture document text:\n{raw}"
    );
    let keys: Vec<&str> = entries
        .first()
        .and_then(|e| e.as_object())
        .map(|obj| obj.keys().map(String::as_str).collect())
        .unwrap_or_default();
    for key in &keys {
        assert!(
            ["category", "term", "count", "mean_confidence"].contains(key),
            "glossary entry has an unexpected field '{key}': {glossary}"
        );
    }
}

/// Regression guard for the config-materialisation gap: `--glossary-out` against a config
/// with no `[glossary]` section loaded must still populate `config.glossary` (so
/// `HaciendaFacade` actually accumulates entries) rather than silently writing `[]`.
#[test]
fn should_materialise_a_glossary_section_when_none_was_configured() {
    let fixture = fixture();
    let path = fixture.path().to_str().expect("utf-8 fixture path");
    let glossary_dir = TempDir::new().expect("create glossary-out directory");
    let glossary_dir_path = glossary_dir.path().to_str().expect("utf-8 dir path");

    // An explicit, empty config file (no [glossary] section) rules out a stray
    // hacienda.toml on the host from accidentally making this test pass for the wrong
    // reason.
    let empty_config = NamedTempFile::with_suffix(".toml").expect("create empty config file");

    let (ok, text) = run(&[
        "--config",
        empty_config.path().to_str().expect("utf-8 config path"),
        "extract",
        "--mode",
        "mask",
        "--glossary-out",
        glossary_dir_path,
        path,
    ]);
    assert!(
        ok,
        "`extract --glossary-out` with no [glossary] section did not exit 0:\n{text}"
    );

    let glossary = read_glossary(&glossary_dir);
    let entries = glossary.as_array().expect("glossary.json is a JSON array");
    assert!(
        !entries.is_empty(),
        "glossary.json was silently empty — the [glossary] materialisation regressed: {glossary}"
    );
}
