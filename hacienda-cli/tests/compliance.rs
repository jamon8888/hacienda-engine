//! `hacienda compliance dpia|model-card|checklist|dora|report`: generates GDPR/AI-Act/DORA
//! artefacts from `[compliance]` configuration (and, for `dora`/`report --incident`, a
//! `PiiIncident` JSON file) — no facade, document, or store involved.

use std::io::Write;
use std::process::Command;
use tempfile::NamedTempFile;

fn run(args: &[&str]) -> (bool, String) {
    let output = Command::new(env!("CARGO_BIN_EXE_hacienda"))
        .args(args)
        .output()
        .expect("failed to run the hacienda binary");
    let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
    text.push_str(&String::from_utf8_lossy(&output.stderr));
    (output.status.success(), text)
}

const COMPLIANCE_CONFIG_JSON: &str = r#"{"compliance":{}}"#;

fn incident_file() -> NamedTempFile {
    let mut file = NamedTempFile::with_suffix(".json").expect("create incident fixture");
    write!(
        file,
        r#"{{
            "summary": "Unredacted spans reached a downstream index",
            "timeline": "detected 10:00, contained 10:20",
            "root_cause": "Model backend disabled by misconfiguration",
            "detected_at": "2026-07-28T10:00:00Z",
            "contained_at": "2026-07-28T10:20:00Z",
            "resolved_at": null,
            "actions_taken": ["Reindexed with redaction enabled"],
            "lessons_learned": ["Fail closed when the model backend is unavailable"]
        }}"#
    )
    .expect("write incident fixture");
    file
}

#[test]
fn should_refuse_when_compliance_is_not_configured() {
    let empty_config = NamedTempFile::with_suffix(".toml").expect("create empty config file");
    let (ok, text) = run(&[
        "--config",
        empty_config.path().to_str().expect("utf-8 config path"),
        "compliance",
        "dpia",
    ]);
    assert!(
        !ok,
        "`compliance dpia` with no [compliance] section exited 0:\n{text}"
    );
    assert!(
        text.to_lowercase().contains("compliance"),
        "the refusal did not name what is missing:\n{text}"
    );
}

#[test]
fn should_generate_a_dpia_naming_the_configured_model() {
    let (ok, text) = run(&[
        "--config-json",
        r#"{"compliance":{"model_name":"gliner2-pii-fr"}}"#,
        "compliance",
        "dpia",
    ]);
    assert!(ok, "`compliance dpia` did not exit 0:\n{text}");
    assert!(
        text.contains("gliner2-pii-fr"),
        "the DPIA does not name the configured model:\n{text}"
    );
}

#[test]
fn should_generate_a_dpia_as_json() {
    let (ok, text) = run(&[
        "--config-json",
        COMPLIANCE_CONFIG_JSON,
        "compliance",
        "dpia",
        "--format",
        "json",
    ]);
    assert!(
        ok,
        "`compliance dpia --format json` did not exit 0:\n{text}"
    );
    let value: serde_json::Value =
        serde_json::from_str(&text).unwrap_or_else(|e| panic!("not valid JSON: {e}\n{text}"));
    assert!(
        value.get("risks").is_some(),
        "DPIA JSON has no risks field: {value}"
    );
}

#[test]
fn should_generate_a_model_card() {
    let (ok, text) = run(&[
        "--config-json",
        r#"{"compliance":{"model_name":"hacienda-pii-test"}}"#,
        "compliance",
        "model-card",
    ]);
    assert!(ok, "`compliance model-card` did not exit 0:\n{text}");
    assert!(
        text.contains("hacienda-pii-test"),
        "unexpected output:\n{text}"
    );
}

#[test]
fn should_generate_a_checklist_covering_all_three_regulations() {
    let (ok, text) = run(&[
        "--config-json",
        COMPLIANCE_CONFIG_JSON,
        "compliance",
        "checklist",
    ]);
    assert!(ok, "`compliance checklist` did not exit 0:\n{text}");
    for label in ["GDPR", "AI Act", "DORA"] {
        assert!(
            text.contains(label),
            "checklist missing {label} section:\n{text}"
        );
    }
}

#[test]
fn should_generate_a_dora_report_from_an_incident_file() {
    let incident = incident_file();
    let (ok, text) = run(&[
        "--config-json",
        r#"{"compliance":{"enabled_reports":["dora"]}}"#,
        "compliance",
        "dora",
        "--incident",
        incident.path().to_str().expect("utf-8 incident path"),
    ]);
    assert!(ok, "`compliance dora` did not exit 0:\n{text}");
    assert!(
        text.contains("PII-INC-"),
        "DORA report does not carry the expected reference prefix:\n{text}"
    );
}

#[test]
fn should_fail_cleanly_on_a_malformed_incident_file() {
    let mut bad = NamedTempFile::with_suffix(".json").expect("create bad incident fixture");
    write!(bad, "not json").expect("write bad fixture");
    let (ok, text) = run(&[
        "--config-json",
        r#"{"compliance":{"enabled_reports":["dora"]}}"#,
        "compliance",
        "dora",
        "--incident",
        bad.path().to_str().expect("utf-8 path"),
    ]);
    assert!(
        !ok,
        "`compliance dora` on a malformed incident file exited 0:\n{text}"
    );
}

#[test]
fn should_omit_dora_from_the_full_report_without_an_incident() {
    let (ok, text) = run(&[
        "--config-json",
        r#"{"compliance":{"enabled_reports":["dora","checklist"]}}"#,
        "compliance",
        "report",
        "--format",
        "json",
    ]);
    assert!(ok, "`compliance report` did not exit 0:\n{text}");
    let value: serde_json::Value =
        serde_json::from_str(&text).unwrap_or_else(|e| panic!("not valid JSON: {e}\n{text}"));
    assert!(
        value["dora"].is_null(),
        "DORA must be omitted from `compliance report` when no --incident is given: {value}"
    );
    assert!(
        value["checklist"].is_object(),
        "checklist should still be present: {value}"
    );
}

#[test]
fn should_include_dora_in_the_full_report_when_an_incident_is_given() {
    let incident = incident_file();
    let (ok, text) = run(&[
        "--config-json",
        r#"{"compliance":{"enabled_reports":["dora"]}}"#,
        "compliance",
        "report",
        "--incident",
        incident.path().to_str().expect("utf-8 incident path"),
        "--format",
        "json",
    ]);
    assert!(ok, "`compliance report --incident` did not exit 0:\n{text}");
    let value: serde_json::Value =
        serde_json::from_str(&text).unwrap_or_else(|e| panic!("not valid JSON: {e}\n{text}"));
    assert!(
        !value["dora"].is_null(),
        "DORA must be present in `compliance report` when --incident is given: {value}"
    );
}
