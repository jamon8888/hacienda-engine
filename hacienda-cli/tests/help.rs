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
fn should_name_audit_append_as_the_concurrency_ceiling() {
    // §6.3: `--concurrency` is bounded by audit-append serialisation, "and that ceiling
    // must be stated in `--help` rather than left to be discovered". A number a user can
    // raise without being told what stops it from helping is a support ticket.
    let (ok, text) = run(&["extract", "--help"]);
    assert!(ok, "`hacienda extract --help` did not exit 0:\n{text}");
    assert!(
        text.contains("--concurrency"),
        "extract --help does not offer --concurrency:\n{text}"
    );
    assert!(
        text.to_lowercase().contains("audit"),
        "extract --help does not name audit append as the ceiling:\n{text}"
    );
}

#[test]
fn should_exit_non_zero_for_an_unknown_command() {
    let (ok, _) = run(&["definitely-not-a-command"]);
    assert!(!ok, "an unknown subcommand exited 0");
}
