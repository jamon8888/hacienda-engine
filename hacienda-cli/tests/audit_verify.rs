//! `hacienda audit verify <dir>`: independently re-verifies the `audit.json` export
//! `extract --audit-out` writes, without needing a live facade, key resolver, or store.

use std::io::Write;
use std::process::Command;
use tempfile::{NamedTempFile, TempDir};

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
fn should_verify_an_audit_chain_written_by_extract() {
    let fixture = fixture();
    let path = fixture.path().to_str().expect("utf-8 fixture path");
    let audit_dir = TempDir::new().expect("create audit-out directory");
    let audit_dir_path = audit_dir.path().to_str().expect("utf-8 audit dir path");

    let (ok, text) = run(&[
        "extract",
        "--mode",
        "mask",
        "--audit-out",
        audit_dir_path,
        path,
    ]);
    assert!(ok, "`extract --audit-out` did not exit 0:\n{text}");

    let (ok, verify_text) = run(&["audit", "verify", audit_dir_path]);
    assert!(
        ok,
        "`audit verify` on a freshly written chain did not exit 0:\n{verify_text}"
    );
    assert!(
        verify_text.to_lowercase().contains("ok") || verify_text.contains("valid"),
        "`audit verify` output does not report success:\n{verify_text}"
    );
}

#[test]
fn should_support_json_format() {
    let fixture = fixture();
    let path = fixture.path().to_str().expect("utf-8 fixture path");
    let audit_dir = TempDir::new().expect("create audit-out directory");
    let audit_dir_path = audit_dir.path().to_str().expect("utf-8 audit dir path");

    run(&[
        "extract",
        "--mode",
        "mask",
        "--audit-out",
        audit_dir_path,
        path,
    ]);

    let (ok, verify_text) = run(&["audit", "verify", "--format", "json", audit_dir_path]);
    assert!(
        ok,
        "`audit verify --format json` did not exit 0:\n{verify_text}"
    );
    let value: serde_json::Value = serde_json::from_str(&verify_text).unwrap_or_else(|e| {
        panic!("`audit verify --format json` did not print valid JSON: {e}\n{verify_text}")
    });
    assert_eq!(value["valid"].as_bool(), Some(true));
    assert!(value.get("audit_chain_tip").is_some());
}

#[test]
fn should_fail_when_audit_json_is_tampered() {
    let fixture = fixture();
    let path = fixture.path().to_str().expect("utf-8 fixture path");
    let audit_dir = TempDir::new().expect("create audit-out directory");
    let audit_dir_path = audit_dir.path().to_str().expect("utf-8 audit dir path");

    let (ok, text) = run(&[
        "extract",
        "--mode",
        "mask",
        "--audit-out",
        audit_dir_path,
        path,
    ]);
    assert!(ok, "`extract --audit-out` did not exit 0:\n{text}");

    let audit_json_path = audit_dir.path().join("audit.json");
    let mut entries: Vec<hacienda::audit::AuditEntry> =
        serde_json::from_slice(&std::fs::read(&audit_json_path).expect("read audit.json"))
            .expect("audit.json is valid JSON");
    assert!(
        !entries.is_empty(),
        "fixture with a detectable email produced no entries"
    );
    entries[0].category = "CreditCard".to_string();
    std::fs::write(
        &audit_json_path,
        serde_json::to_vec_pretty(&entries).expect("re-serialise tampered entries"),
    )
    .expect("write tampered audit.json");

    let (ok, verify_text) = run(&["audit", "verify", audit_dir_path]);
    assert!(
        !ok,
        "`audit verify` on a tampered chain exited 0 instead of failing:\n{verify_text}"
    );
}

#[test]
fn should_fail_cleanly_when_audit_json_is_missing() {
    let empty_dir = TempDir::new().expect("create empty directory");
    let (ok, text) = run(&[
        "audit",
        "verify",
        empty_dir.path().to_str().expect("utf-8 dir path"),
    ]);
    assert!(
        !ok,
        "`audit verify` on a directory with no audit.json exited 0:\n{text}"
    );
    assert!(
        !text.is_empty(),
        "`audit verify` on a missing audit.json produced no diagnostic at all"
    );
}
