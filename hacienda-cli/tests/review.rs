//! `hacienda review list|show|assign|decide|stats <dir>`: operates on the durable
//! `FileReviewStore` a preceding `extract --review-out <dir>` run wrote.

use std::io::Write;
use std::process::Command;
use tempfile::{NamedTempFile, TempDir};

/// Low-confidence-looking text: a bare digit string is not confidently an email or phone
/// number under the regex-first detector's default threshold, so it is more likely to be
/// routed to review than accepted outright. The review pipeline's own confidence math is
/// exercised elsewhere (`hacienda-core::review::queue` tests); this fixture only needs to
/// reliably produce *some* PII entities to seed a queue with.
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

/// Force every detection below the review threshold, regardless of confidence, so this
/// suite does not depend on the model's precise scoring. Regex-sourced detections carry
/// confidence exactly `1.0` (`hacienda_core::pii::types`), and `ReviewQueue::needs_review`
/// is a strict `<` comparison, so the threshold must be set *above* 1.0 — `1.0` itself
/// would never route anything.
fn seed_review_queue(review_dir: &TempDir, fixture_path: &str) {
    let (ok, text) = run(&[
        "extract",
        "--mode",
        "mask",
        "--config-json",
        r#"{"review":{"confidence_threshold":1.5}}"#,
        "--review-out",
        review_dir.path().to_str().expect("utf-8 dir path"),
        fixture_path,
    ]);
    assert!(ok, "`extract --review-out` did not exit 0:\n{text}");
}

#[test]
fn should_list_items_seeded_by_extract_review_out() {
    let fixture = fixture();
    let path = fixture.path().to_str().expect("utf-8 fixture path");
    let review_dir = TempDir::new().expect("create review-out directory");
    seed_review_queue(&review_dir, path);

    let (ok, text) = run(&[
        "review",
        "list",
        review_dir.path().to_str().expect("utf-8 dir path"),
    ]);
    assert!(ok, "`review list` did not exit 0:\n{text}");
    assert!(
        text.contains("Email"),
        "`review list` did not show the seeded Email item:\n{text}"
    );
}

#[test]
fn should_round_trip_through_show_assign_and_decide() {
    let fixture = fixture();
    let path = fixture.path().to_str().expect("utf-8 fixture path");
    let review_dir = TempDir::new().expect("create review-out directory");
    seed_review_queue(&review_dir, path);
    let dir_path = review_dir.path().to_str().expect("utf-8 dir path");

    let (ok, list_json) = run(&["review", "list", "--format", "json", dir_path]);
    assert!(
        ok,
        "`review list --format json` did not exit 0:\n{list_json}"
    );
    let items: serde_json::Value =
        serde_json::from_str(&list_json).expect("`review list --format json` printed valid JSON");
    let id = items[0]["id"]
        .as_str()
        .expect("seeded review queue has at least one item")
        .to_string();

    let (ok, show_text) = run(&["review", "show", dir_path, &id]);
    assert!(ok, "`review show` did not exit 0:\n{show_text}");
    assert!(
        show_text.contains(&id),
        "`review show` omitted the id:\n{show_text}"
    );

    let (ok, assign_text) = run(&["review", "assign", dir_path, &id, "--reviewer", "alice"]);
    assert!(ok, "`review assign` did not exit 0:\n{assign_text}");
    assert!(
        assign_text.contains("in_review"),
        "`review assign` did not report the item as in_review:\n{assign_text}"
    );

    let (ok, decide_text) = run(&[
        "review",
        "decide",
        dir_path,
        &id,
        "--decision",
        "approve",
        "--reviewer",
        "alice",
        "--comment",
        "looks right",
    ]);
    assert!(ok, "`review decide` did not exit 0:\n{decide_text}");
    assert!(
        decide_text.contains("approved"),
        "`review decide` did not report the item as approved:\n{decide_text}"
    );

    // The durable store persisted the decision — a fresh `review show` invocation (a
    // brand new process, no shared in-memory state) must see it too.
    let (ok, show_again) = run(&["review", "show", "--format", "json", dir_path, &id]);
    assert!(ok, "second `review show` did not exit 0:\n{show_again}");
    let value: serde_json::Value =
        serde_json::from_str(&show_again).expect("`review show --format json` printed valid JSON");
    assert_eq!(value["status"], "approved");
    assert_eq!(value["decided_by"], "alice");
}

#[test]
fn should_refuse_a_second_decision_on_the_same_item() {
    let fixture = fixture();
    let path = fixture.path().to_str().expect("utf-8 fixture path");
    let review_dir = TempDir::new().expect("create review-out directory");
    seed_review_queue(&review_dir, path);
    let dir_path = review_dir.path().to_str().expect("utf-8 dir path");

    let (_, list_json) = run(&["review", "list", "--format", "json", dir_path]);
    let items: serde_json::Value = serde_json::from_str(&list_json).expect("valid JSON");
    let id = items[0]["id"].as_str().expect("has an item").to_string();

    let (ok, _) = run(&[
        "review",
        "decide",
        dir_path,
        &id,
        "--decision",
        "approve",
        "--reviewer",
        "alice",
    ]);
    assert!(ok, "first decide should succeed");

    let (ok, text) = run(&[
        "review",
        "decide",
        dir_path,
        &id,
        "--decision",
        "reject",
        "--reviewer",
        "bob",
    ]);
    assert!(
        !ok,
        "deciding an already-decided item exited 0 instead of failing:\n{text}"
    );
    assert!(
        text.to_lowercase().contains("already"),
        "the failure did not explain that the item was already decided:\n{text}"
    );
}

#[test]
fn should_report_zero_stats_on_an_empty_directory_rather_than_failing() {
    let empty_dir = TempDir::new().expect("create empty directory");
    let (ok, text) = run(&[
        "review",
        "stats",
        empty_dir.path().to_str().expect("utf-8 dir path"),
    ]);
    assert!(
        ok,
        "`review stats` on a directory with no review.jsonl yet did not exit 0:\n{text}"
    );
    assert!(
        text.contains("total:     0"),
        "`review stats` on an empty store did not report zero items:\n{text}"
    );
}

#[test]
fn should_fail_cleanly_when_the_item_id_does_not_exist() {
    let empty_dir = TempDir::new().expect("create empty directory");
    let (ok, text) = run(&[
        "review",
        "show",
        empty_dir.path().to_str().expect("utf-8 dir path"),
        "no-such-id",
    ]);
    assert!(!ok, "`review show` on an unknown id exited 0:\n{text}");
    assert!(!text.is_empty(), "the failure produced no diagnostic");
}

#[test]
fn should_accumulate_across_repeated_extract_review_out_runs() {
    let fixture = fixture();
    let path = fixture.path().to_str().expect("utf-8 fixture path");
    let review_dir = TempDir::new().expect("create review-out directory");
    seed_review_queue(&review_dir, path);
    seed_review_queue(&review_dir, path);

    let (ok, stats_text) = run(&[
        "review",
        "stats",
        review_dir.path().to_str().expect("utf-8 dir path"),
    ]);
    assert!(ok, "`review stats` did not exit 0:\n{stats_text}");
    assert!(
        stats_text.contains("total:     2"),
        "two `--review-out` runs against the same directory should accumulate to 2 items, \
         not overwrite each other:\n{stats_text}"
    );
}
