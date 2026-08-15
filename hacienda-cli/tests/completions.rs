//! `hacienda completions <shell>`: prints a shell completion script generated from the
//! same `Cli` clap parses with — see `commands::run_completions`.

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
fn should_generate_a_bash_completion_script_naming_the_new_subcommands() {
    let (ok, text) = run(&["completions", "bash"]);
    assert!(ok, "`completions bash` did not exit 0:\n{text}");
    assert!(
        text.contains("hacienda"),
        "bash completion script does not mention hacienda:\n{text}"
    );
    for subcommand in ["review", "compliance", "audit"] {
        assert!(
            text.contains(subcommand),
            "bash completion script does not mention `{subcommand}`:\n{text}"
        );
    }
}

#[test]
fn should_generate_completion_scripts_for_every_supported_shell() {
    for shell in ["bash", "zsh", "fish", "powershell", "elvish"] {
        let (ok, text) = run(&["completions", shell]);
        assert!(ok, "`completions {shell}` did not exit 0:\n{text}");
        assert!(!text.is_empty(), "`completions {shell}` printed nothing");
    }
}

#[test]
fn should_reject_an_unknown_shell() {
    let (ok, _) = run(&["completions", "not-a-real-shell"]);
    assert!(!ok, "`completions not-a-real-shell` exited 0");
}
