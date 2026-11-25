//! End-to-end checks for `hacienda claude install`: the command is wired into the
//! binary, unrelated user settings survive the merge, and the gateway credential is
//! delivered without ever being hardcoded or written to a world-readable file.

use std::fs;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::process::Command;

fn hacienda(args: &[&str]) -> (bool, String) {
    let output = Command::new(env!("CARGO_BIN_EXE_hacienda"))
        .args(args)
        .output()
        .expect("failed to run the hacienda binary");
    let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
    text.push_str(&String::from_utf8_lossy(&output.stderr));
    (output.status.success(), text)
}

#[test]
fn claude_install_is_wired_and_documents_its_flags() {
    let (ok, help) = hacienda(&["--help"]);
    assert!(ok, "`hacienda --help` failed:\n{help}");
    assert!(
        help.contains("claude"),
        "top-level help does not name `claude`:\n{help}"
    );

    let (ok, help) = hacienda(&["claude", "install", "--help"]);
    assert!(ok, "`hacienda claude install --help` failed:\n{help}");
    for flag in [
        "--settings",
        "--gateway-url",
        "--credential-mode",
        "--credential-env",
        "--credential-file",
    ] {
        assert!(
            help.contains(flag),
            "claude install --help does not show {flag}:\n{help}"
        );
    }
}

#[cfg(unix)]
#[test]
fn helper_mode_install_keeps_the_secret_out_of_settings_and_uses_private_modes() {
    let dir = tempfile::tempdir().expect("temp dir");
    let settings = dir.path().join("settings.json");
    let credential = dir.path().join("credential");
    let secret = "hcd_documents:process_cli_secret_value";
    fs::write(&credential, format!("{secret}\n")).expect("write credential source");

    let settings_str = settings.to_string_lossy().to_string();
    let credential_str = credential.to_string_lossy().to_string();
    let (ok, output) = hacienda(&[
        "claude",
        "install",
        "--settings",
        &settings_str,
        "--credential-mode",
        "helper",
        "--credential-file",
        &credential_str,
        "--gateway-url",
        "http://127.0.0.1:8787",
    ]);
    assert!(ok, "claude install (helper mode) failed:\n{output}");

    let document = fs::read_to_string(&settings).expect("settings.json written");
    assert!(
        !document.contains(secret),
        "settings.json leaked the credential: {document}"
    );
    assert!(document.contains("ANTHROPIC_BASE_URL"));
    assert!(document.contains("http://127.0.0.1:8787"));
    assert!(document.contains("apiKeyHelper"));
    assert!(
        !output.contains(secret),
        "installer stdout/stderr leaked the credential"
    );

    let settings_mode = fs::metadata(&settings).unwrap().permissions().mode() & 0o777;
    assert_eq!(settings_mode, 0o600, "settings.json must be owner-only");
    let secret_file = dir.path().join("hacienda-gateway-token");
    let secret_mode = fs::metadata(&secret_file).unwrap().permissions().mode() & 0o777;
    assert_eq!(secret_mode, 0o600, "secret file must be owner-only");
    let helper = dir.path().join("hacienda-gateway-key.sh");
    let helper_mode = fs::metadata(&helper).unwrap().permissions().mode() & 0o777;
    assert_eq!(
        helper_mode, 0o700,
        "credential helper must be executable by owner only"
    );
    let helper_script = fs::read_to_string(&helper).unwrap();
    assert!(
        !helper_script.contains(secret),
        "helper script must not embed the credential"
    );
}

#[cfg(unix)]
#[test]
fn env_mode_install_preserves_unrelated_settings() {
    let dir = tempfile::tempdir().expect("temp dir");
    let settings = dir.path().join("settings.json");
    let credential = dir.path().join("credential");
    let secret = "hcd_documents:process_env_mode_secret";
    fs::write(&credential, secret).expect("write credential source");
    let existing = r#"{
      "$schema": "https://json.schemastore.org/claude-code-settings.json",
      "permissions": { "allow": ["Bash(cargo test *)"] },
      "env": { "KEEP": "yes" }
    }"#;
    fs::write(&settings, existing).expect("seed settings.json");

    let settings_str = settings.to_string_lossy().to_string();
    let credential_str = credential.to_string_lossy().to_string();
    let (ok, output) = hacienda(&[
        "claude",
        "install",
        "--settings",
        &settings_str,
        "--credential-mode",
        "env",
        "--credential-file",
        &credential_str,
    ]);
    assert!(ok, "claude install (env mode) failed:\n{output}");

    let document = fs::read_to_string(&settings).expect("settings.json written");
    let value: serde_json::Value =
        serde_json::from_str(&document).expect("merged settings is JSON");
    assert_eq!(
        value.get("permissions").unwrap().get("allow").unwrap()[0],
        "Bash(cargo test *)"
    );
    let env = value.get("env").unwrap().as_object().unwrap();
    assert_eq!(env.get("KEEP").unwrap().as_str().unwrap(), "yes");
    assert_eq!(
        env.get("ANTHROPIC_AUTH_TOKEN").unwrap().as_str().unwrap(),
        secret
    );
    assert_eq!(
        env.get("ANTHROPIC_BASE_URL").unwrap().as_str().unwrap(),
        "http://127.0.0.1:8787"
    );
    assert_eq!(
        fs::metadata(&settings).unwrap().permissions().mode() & 0o777,
        0o600
    );
    assert!(
        !output.contains(secret),
        "installer must not print the credential"
    );
}

#[test]
fn install_fails_closed_without_a_credential_source() {
    let dir = tempfile::tempdir().expect("temp dir");
    let settings = dir.path().join("settings.json");
    let settings_str = settings.to_string_lossy().to_string();
    let missing_env = "HACIENDA_TEST_MISSING_CLAUDE_CREDENTIAL_XYZ";
    let output = Command::new(env!("CARGO_BIN_EXE_hacienda"))
        .args([
            "claude",
            "install",
            "--settings",
            &settings_str,
            "--credential-env",
            missing_env,
        ])
        .env_remove(missing_env)
        .output()
        .expect("failed to run the hacienda binary");
    assert!(
        !output.status.success(),
        "install without a credential must fail"
    );
    assert!(
        !settings.exists(),
        "no settings file should be written on failure"
    );
    let text = String::from_utf8_lossy(&output.stderr);
    assert!(
        text.contains("credential"),
        "error should name the missing credential:\n{text}"
    );
}
