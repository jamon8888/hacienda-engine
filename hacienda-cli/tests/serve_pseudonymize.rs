//! `hacienda serve` must build its facade the same way `extract`/`scan` do: with a
//! pseudonym key resolver attached whenever `[pii.redaction] mode = "pseudonymize"`.
//!
//! `HaciendaFacade::build` resolves the pseudonymiser eagerly (`PiiPipeline::with_pseudonymiser`
//! → `RedactionEngine::new`, propagated through `.transpose()?`), so a facade built without
//! one for pseudonymize mode fails at *construction*, not at first request — before the fix,
//! `run_serve` called `HaciendaFacade::new(config)` directly and the server never got past
//! that point: it exited before printing its "serving on" line. This is regression-tested by
//! process behaviour alone, no HTTP client needed.

use std::io::{BufRead, BufReader};
use std::io::Write;
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;
use tempfile::NamedTempFile;

const KEY_ID: &str = "k1";
const KEY_BYTES: usize = 64;
const KEY_HEX: &str = "aa";

fn key_hex() -> String {
    KEY_HEX.repeat(KEY_BYTES)
}

fn pseudonymize_config() -> NamedTempFile {
    let mut file = NamedTempFile::with_suffix(".toml").expect("create config file");
    writeln!(file, "[pii.redaction]\nmode = \"pseudonymize\"\n").expect("write config");
    file
}

/// Spawn `hacienda --config <config_path> serve --bind 127.0.0.1:0` with the pseudonym key
/// env vars set, wait up to 5s for either its "serving on" stderr line or process exit
/// (whichever comes first), then kill it. Returns `Some(line)` if the server started,
/// `None` if the process exited first (the pre-fix failure mode).
fn try_start_server(config_path: &str) -> Option<String> {
    let mut child = Command::new(env!("CARGO_BIN_EXE_hacienda"))
        .args(["--config", config_path, "serve", "--bind", "127.0.0.1:0"])
        .env("HACIENDA_PSEUDONYM_ACTIVE_KEY", KEY_ID)
        .env(
            format!("HACIENDA_PSEUDONYM_KEY_{}", KEY_ID.to_uppercase()),
            key_hex(),
        )
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .expect("failed to spawn the hacienda binary");

    let stderr = child.stderr.take().expect("piped stderr");
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let reader = BufReader::new(stderr);
        for line in reader.lines().map_while(Result::ok) {
            let is_serving = line.contains("hacienda serving on");
            let _ = tx.send(line);
            if is_serving {
                return;
            }
        }
        // Reader hit EOF (process exited) without ever printing the "serving" line.
        let _ = tx.send(String::from("<eof: process exited without serving>"));
    });

    let deadline = Duration::from_secs(5);
    let start = std::time::Instant::now();
    let mut serving_line = None;
    while start.elapsed() < deadline {
        match rx.recv_timeout(deadline.saturating_sub(start.elapsed())) {
            Ok(line) if line.contains("hacienda serving on") => {
                serving_line = Some(line);
                break;
            }
            Ok(line) if line.starts_with("<eof:") => break,
            Ok(_other_line) => continue,
            Err(_timeout) => break,
        }
    }

    let _ = child.kill();
    let _ = child.wait();
    serving_line
}

#[test]
fn should_start_serving_in_pseudonymize_mode_with_a_key_configured() {
    let config = pseudonymize_config();
    let config_path = config.path().to_str().expect("utf-8 config path");

    let serving_line = try_start_server(config_path);
    assert!(
        serving_line.is_some(),
        "`hacienda serve` in pseudonymize mode with a key configured did not start — \
         this is the regression this test guards: run_serve must build its facade via \
         build_facade (mode-conditional key resolver), not HaciendaFacade::new directly"
    );
}
