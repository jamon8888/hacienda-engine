//! Task 2 Step 2 of Phase 2: the binary exists, runs, and names the commands this phase
//! ships.
//!
//! `CARGO_BIN_EXE_hacienda` is set by cargo for integration tests in the crate that
//! defines the binary, so this needs no test-harness dependency.

use std::process::Command;

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
fn should_list_this_phase_s_commands_in_help() {
    let (ok, text) = run(&["--help"]);
    assert!(ok, "`hacienda --help` did not exit 0:\n{text}");
    for command in ["extract", "scan", "config"] {
        assert!(
            text.contains(command),
            "--help does not name `{command}`:\n{text}"
        );
    }
}

#[test]
fn should_document_the_measured_concurrency_ceiling() {
    // §6.3 / §9 (Task 5, issue #31): the Phase 2 measurement showed raising
    // --concurrency does not reliably scale throughput on constrained hardware, and
    // that the audit store's `io_order` lock is *not* why (measured wait stayed under
    // 0.2% of wall time at every concurrency level tested). A flag a user can raise
    // without being told it likely won't help, and why, is a support ticket — "that
    // ceiling must be stated in `--help` rather than left to be discovered".
    let (ok, text) = run(&["extract", "--help"]);
    assert!(ok, "`hacienda extract --help` did not exit 0:\n{text}");
    assert!(
        text.contains("--concurrency"),
        "extract --help does not offer --concurrency:\n{text}"
    );
    let lower = text.to_lowercase();
    assert!(
        lower.contains("audit"),
        "extract --help does not mention the audit lock finding:\n{text}"
    );
    assert!(
        lower.contains("throughput"),
        "extract --help does not state the measured throughput ceiling:\n{text}"
    );
}

#[test]
fn should_exit_non_zero_for_an_unknown_command() {
    let (ok, _) = run(&["definitely-not-a-command"]);
    assert!(!ok, "an unknown subcommand exited 0");
}
