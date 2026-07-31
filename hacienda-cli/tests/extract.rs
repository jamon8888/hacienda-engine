//! Task 6 of Phase 2: `extract` never lets raw PII reach stdout, `--no-redact` refuses
//! without explicit acknowledgement and names `scan` as the honest alternative (D5), and
//! `--audit-out` writes a chain that independently re-verifies.

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
    run_without_pseudonym_key(args)
}

/// `HACIENDA_PSEUDONYM_ACTIVE_KEY` is process-global; explicitly clearing it keeps the
/// pseudonymize-without-a-key assertion from depending on whatever the host happens to
/// have set, which is what makes that assertion trustworthy rather than accidental.
fn run_without_pseudonym_key(args: &[&str]) -> (bool, String) {
    let output = Command::new(env!("CARGO_BIN_EXE_hacienda"))
        .args(args)
        .env_remove("HACIENDA_PSEUDONYM_ACTIVE_KEY")
        .output()
        .expect("failed to run the hacienda binary");
    let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
    text.push_str(&String::from_utf8_lossy(&output.stderr));
    (output.status.success(), text)
}

#[test]
fn should_never_emit_the_known_email_in_mask_mode() {
    let fixture = fixture();
    let path = fixture.path().to_str().expect("utf-8 fixture path");

    let (ok, text) = run(&["extract", "--mode", "mask", path]);

    assert!(ok, "`extract --mode mask` did not exit 0:\n{text}");
    assert!(
        !text.contains(KNOWN_EMAIL),
        "mask mode leaked the known email address:\n{text}"
    );
    assert!(
        text.contains("[EMAIL]") || text.to_lowercase().contains("email"),
        "mask mode output does not show a redaction/detection marker:\n{text}"
    );
}

#[test]
fn should_never_emit_the_known_email_in_hash_mode() {
    let fixture = fixture();
    let path = fixture.path().to_str().expect("utf-8 fixture path");

    let (ok, text) = run(&["extract", "--mode", "hash", path]);

    assert!(ok, "`extract --mode hash` did not exit 0:\n{text}");
    assert!(
        !text.contains(KNOWN_EMAIL),
        "hash mode leaked the known email address:\n{text}"
    );
}

#[test]
fn should_refuse_pseudonymize_mode_without_a_key_rather_than_degrade() {
    let fixture = fixture();
    let path = fixture.path().to_str().expect("utf-8 fixture path");

    let (ok, text) = run(&["extract", "--mode", "pseudonymize", path]);

    assert!(
        !ok,
        "`extract --mode pseudonymize` with no key exited 0 instead of refusing:\n{text}"
    );
    assert!(
        !text.contains(KNOWN_EMAIL),
        "pseudonymize mode leaked the known email address while refusing:\n{text}"
    );
}

#[test]
fn should_refuse_no_redact_without_explicit_acknowledgement_and_name_scan() {
    let fixture = fixture();
    let path = fixture.path().to_str().expect("utf-8 fixture path");

    let (ok, text) = run(&["extract", "--no-redact", path]);

    assert!(
        !ok,
        "`extract --no-redact` without acknowledgement exited 0:\n{text}"
    );
    let lower = text.to_lowercase();
    assert!(
        lower.contains("i-accept-unredacted-pii"),
        "refusal does not name the acknowledgement flag:\n{text}"
    );
    assert!(
        lower.contains("scan"),
        "refusal does not name `scan` as the alternative (D5):\n{text}"
    );
    assert!(
        !text.contains(KNOWN_EMAIL),
        "refused --no-redact still leaked the known email address:\n{text}"
    );
}

#[test]
fn should_allow_no_redact_with_explicit_acknowledgement() {
    let fixture = fixture();
    let path = fixture.path().to_str().expect("utf-8 fixture path");

    let (ok, text) = run(&["extract", "--no-redact", "--i-accept-unredacted-pii", path]);

    assert!(
        ok,
        "`extract --no-redact --i-accept-unredacted-pii` did not exit 0:\n{text}"
    );
    assert!(
        text.contains(KNOWN_EMAIL),
        "acknowledged --no-redact did not emit the raw document text:\n{text}"
    );
}

#[test]
fn should_write_an_audit_chain_that_independently_re_verifies() {
    let fixture = fixture();
    let path = fixture.path().to_str().expect("utf-8 fixture path");
    let audit_dir = TempDir::new().expect("create audit-out directory");

    let (ok, text) = run(&[
        "extract",
        "--mode",
        "mask",
        "--audit-out",
        audit_dir.path().to_str().expect("utf-8 audit dir path"),
        path,
    ]);
    assert!(ok, "`extract --audit-out` did not exit 0:\n{text}");

    let audit_path = audit_dir.path().join("audit.json");
    let bytes = std::fs::read(&audit_path)
        .unwrap_or_else(|e| panic!("audit.json was not written at {audit_path:?}: {e}"));
    let entries: Vec<hacienda::audit::AuditEntry> = serde_json::from_slice(&bytes)
        .unwrap_or_else(|e| panic!("audit.json is not a valid entry array: {e}"));
    assert!(
        !entries.is_empty(),
        "audit.json contains no entries for a fixture with a detectable email"
    );

    // Rebuild the chain from exactly what was written and re-verify it independently of
    // the process that wrote it — `AuditChain::append` recomputes and checks each
    // entry's chain hash, so this fails if the file merely exists but does not verify.
    let config_hash = entries[0].config_hash.clone();
    let mut chain = hacienda::audit::AuditChain::new(config_hash);
    for entry in entries {
        chain
            .append(entry)
            .expect("a written audit entry failed to re-verify against the chain");
    }
    chain
        .verify()
        .expect("the reconstructed audit chain failed full verification");
}

/// Track J1: `--vault <dir>` emits the Track I2 layout — `documents/`, `_manifest.json`,
/// `pii-registry.json`, `README.md` — and never leaks the raw email into a redacted
/// document, same contract as stdout output.
#[test]
fn should_write_a_vault_with_a_redacted_document_and_pii_registry() {
    let fixture = fixture();
    let path = fixture.path().to_str().expect("utf-8 fixture path");
    let stem = fixture
        .path()
        .file_stem()
        .and_then(|s| s.to_str())
        .expect("fixture has a utf-8 stem")
        .to_string();
    let vault_dir = TempDir::new().expect("create vault directory");

    let (ok, text) = run(&[
        "extract",
        "--mode",
        "mask",
        "--vault",
        vault_dir.path().to_str().expect("utf-8 vault dir path"),
        path,
    ]);
    assert!(ok, "`extract --vault` did not exit 0:\n{text}");
    eprintln!("stderr/stdout was: {text}");

    let doc_path = vault_dir.path().join("documents").join(format!("{stem}.md"));
    let markdown = std::fs::read_to_string(&doc_path)
        .unwrap_or_else(|e| panic!("vault document was not written at {doc_path:?}: {e}"));
    assert!(
        !markdown.contains(KNOWN_EMAIL),
        "vault document leaked the known email address:\n{markdown}"
    );
    assert!(
        markdown.contains("pii_entities_found: 1"),
        "vault document frontmatter does not record the PII count:\n{markdown}"
    );

    let manifest: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(vault_dir.path().join("_manifest.json"))
            .expect("_manifest.json was not written"),
    )
    .expect("_manifest.json is not valid JSON");
    assert_eq!(manifest["files"].as_array().map(Vec::len), Some(1));

    let registry: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(vault_dir.path().join("pii-registry.json"))
            .expect("pii-registry.json was not written"),
    )
    .expect("pii-registry.json is not valid JSON");
    let registry_text = registry.to_string();
    assert!(
        !registry_text.contains(KNOWN_EMAIL),
        "pii-registry.json leaked the known email address:\n{registry_text}"
    );
    assert_eq!(registry.as_array().map(Vec::len), Some(1));

    let readme = std::fs::read_to_string(vault_dir.path().join("README.md"))
        .expect("README.md was not written");
    assert!(
        readme.contains("does not include an audit chain"),
        "README does not say --audit-out was omitted:\n{readme}"
    );
    assert!(
        !vault_dir.path().join("entities").exists(),
        "Track J2 decided this vault is thinner than Studio's — no entities/ directory"
    );
    assert!(
        !vault_dir.path().join("GLOSSARY.md").exists(),
        "Track J2 decided this vault is thinner than Studio's — no GLOSSARY.md"
    );
}

/// Track J3: when `--audit-out` and `--vault` are both given, the vault carries its own
/// copy of the audit chain rather than leaving the consumer to find a second directory.
#[test]
fn should_mirror_the_audit_chain_into_the_vault_when_both_flags_are_given() {
    let fixture = fixture();
    let path = fixture.path().to_str().expect("utf-8 fixture path");
    let audit_dir = TempDir::new().expect("create audit-out directory");
    let vault_dir = TempDir::new().expect("create vault directory");

    let (ok, text) = run(&[
        "extract",
        "--mode",
        "mask",
        "--audit-out",
        audit_dir.path().to_str().expect("utf-8 audit dir path"),
        "--vault",
        vault_dir.path().to_str().expect("utf-8 vault dir path"),
        path,
    ]);
    assert!(ok, "`extract --audit-out --vault` did not exit 0:\n{text}");

    let mirrored = vault_dir.path().join("audit").join("audit.json");
    assert!(
        mirrored.exists(),
        "audit chain was not mirrored into the vault at {mirrored:?}"
    );

    let readme = std::fs::read_to_string(vault_dir.path().join("README.md"))
        .expect("README.md was not written");
    assert!(
        readme.contains("includes an audit chain"),
        "README does not say the vault includes an audit chain:\n{readme}"
    );
}
