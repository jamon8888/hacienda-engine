//! `hacienda audit list|export <dir> --node <id>`: reads a durable, segmented
//! `FileAuditStore` — distinct from `audit verify`, which reads the flat `audit.json`
//! export `--audit-out` writes. Nothing in this CLI writes that segmented layout yet (see
//! `AuditCommand::List`'s doc comment), so these tests seed one directly via
//! `hacienda::audit::FileAuditStore`, the same way a library caller embedding
//! `hacienda-core` would.

use hacienda::audit::{
    AuditEntryInput, AuditStore, EntitySource, FileAuditStore, NodeId, RedactionAction,
};
use hacienda::hacienda_core::TenantId;
use std::process::Command;
use tempfile::TempDir;

fn run(args: &[&str]) -> (bool, String) {
    let output = Command::new(env!("CARGO_BIN_EXE_hacienda"))
        .args(args)
        .output()
        .expect("failed to run the hacienda binary");
    let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
    text.push_str(&String::from_utf8_lossy(&output.stderr));
    (output.status.success(), text)
}

fn entry_input(id: &str) -> AuditEntryInput {
    AuditEntryInput {
        id: id.to_string(),
        category: "Email".to_string(),
        action: RedactionAction::Mask,
        span_hash: blake3::hash(id.as_bytes()).to_hex().to_string(),
        span_length: 10,
        confidence: Some(0.9),
        source: EntitySource::Regex,
        pipeline_version: "test".to_string(),
        config_hash: String::new(),
        principal: None,
        vertical: None,
    }
}

/// Seed a `FileAuditStore` at `dir/<node>/` with `count` entries, sealing on drop.
async fn seed(dir: &TempDir, node: &str, config_hash: &str, count: usize) {
    let store = FileAuditStore::open(dir.path(), NodeId::new(node), config_hash)
        .expect("open a fresh FileAuditStore");
    let inputs = (0..count)
        .map(|i| entry_input(&format!("id-{i}")))
        .collect();
    store
        .append(&TenantId::default_tenant(), inputs)
        .await
        .expect("append seed entries");
    store
        .close(&TenantId::default_tenant())
        .await
        .expect("seal the segment");
}

#[tokio::test]
async fn should_list_entries_seeded_directly_via_file_audit_store() {
    let dir = TempDir::new().expect("create audit store directory");
    seed(&dir, "test-node", "cfg-hash", 3).await;

    let (ok, text) = run(&[
        "audit",
        "list",
        "--node",
        "test-node",
        "--config-hash",
        "cfg-hash",
        dir.path().to_str().expect("utf-8 dir path"),
    ]);
    assert!(ok, "`audit list` did not exit 0:\n{text}");
    let email_count = text.matches("[Email]").count();
    assert_eq!(
        email_count, 3,
        "`audit list` should show all 3 seeded entries under a default limit of 50:\n{text}"
    );
    // A non-empty page always carries a `next` cursor (see `AuditPage::next`'s own doc:
    // "this page was empty", not "the history is finished") — a caller finds the true end
    // only by requesting another page and getting nothing back, which `audit list` does
    // not automatically loop on. So the assertion here is that a cursor is printed at all,
    // not that the store claims to be exhausted.
    assert!(
        text.contains("next:"),
        "`audit list` did not print a resumable cursor:\n{text}"
    );
}

#[tokio::test]
async fn should_page_with_after_and_limit() {
    let dir = TempDir::new().expect("create audit store directory");
    seed(&dir, "test-node", "cfg-hash", 3).await;
    let dir_path = dir.path().to_str().expect("utf-8 dir path");

    let (ok, page1) = run(&[
        "audit",
        "list",
        "--node",
        "test-node",
        "--config-hash",
        "cfg-hash",
        "--limit",
        "2",
        "--format",
        "json",
        dir_path,
    ]);
    assert!(ok, "first page did not exit 0:\n{page1}");
    let page1: serde_json::Value = serde_json::from_str(&page1).expect("valid JSON");
    assert_eq!(page1["entries"].as_array().unwrap().len(), 2, "{page1}");
    let cursor = page1["next"]
        .as_str()
        .expect("a 2-of-3 page has a next cursor")
        .to_string();

    let (ok, page2) = run(&[
        "audit",
        "list",
        "--node",
        "test-node",
        "--config-hash",
        "cfg-hash",
        "--limit",
        "2",
        "--after",
        &cursor,
        "--format",
        "json",
        dir_path,
    ]);
    assert!(ok, "second page did not exit 0:\n{page2}");
    let page2: serde_json::Value = serde_json::from_str(&page2).expect("valid JSON");
    assert_eq!(
        page2["entries"].as_array().unwrap().len(),
        1,
        "the remaining entry should be exactly 1: {page2}"
    );
}

#[tokio::test]
async fn should_export_to_json_csv_and_jsonl() {
    let dir = TempDir::new().expect("create audit store directory");
    seed(&dir, "test-node", "cfg-hash", 2).await;
    let dir_path = dir.path().to_str().expect("utf-8 dir path");

    for (format, suffix) in [("json", "json"), ("csv", "csv"), ("json-lines", "jsonl")] {
        let out_dir = TempDir::new().expect("create export output directory");
        let out = out_dir.path().join(format!("export.{suffix}"));
        let (ok, text) = run(&[
            "audit",
            "export",
            "--node",
            "test-node",
            "--config-hash",
            "cfg-hash",
            "--format",
            format,
            "--out",
            out.to_str().expect("utf-8 out path"),
            dir_path,
        ]);
        assert!(
            ok,
            "`audit export --format {format}` did not exit 0:\n{text}"
        );
        let bytes =
            std::fs::read(&out).unwrap_or_else(|e| panic!("export file was not written: {e}"));
        assert!(
            !bytes.is_empty(),
            "`audit export --format {format}` wrote an empty file"
        );
    }
}

#[tokio::test]
async fn should_report_no_entries_on_an_empty_store_rather_than_failing() {
    let dir = TempDir::new().expect("create empty audit store directory");
    let (ok, text) = run(&[
        "audit",
        "list",
        "--node",
        "fresh-node",
        dir.path().to_str().expect("utf-8 dir path"),
    ]);
    assert!(
        ok,
        "`audit list` on a brand-new node directory did not exit 0:\n{text}"
    );
    assert!(
        text.contains("no entries"),
        "`audit list` on an empty store should say so, not print nothing:\n{text}"
    );
}

#[test]
fn should_fail_cleanly_when_out_path_is_unwritable() {
    let dir = TempDir::new().expect("create audit store directory");
    let (ok, text) = run(&[
        "audit",
        "export",
        "--node",
        "test-node",
        "--out",
        "/nonexistent-directory-for-hacienda-tests/export.json",
        dir.path().to_str().expect("utf-8 dir path"),
    ]);
    assert!(
        !ok,
        "`audit export` to an unwritable path exited 0 instead of failing:\n{text}"
    );
    assert!(!text.is_empty(), "the failure produced no diagnostic");
}
