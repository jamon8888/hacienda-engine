//! P6 exit criterion: `2026-08-13-P6-llm-call-enforcement-point.md` §6, third bullet.
//!
//! "A structural/compile-time test (or, if Rust's type system can't express it directly,
//! a workspace-wide grep-based CI check, the same fallback P1 itself may need to use)
//! asserts no crate outside the guard's owning module constructs a `liter_llm::LlmClient`
//! or calls `xberg::llm::{complete_text,complete_with_json_schema}` directly."
//!
//! `GuardedLlm` (`src/stream.rs`) is the guard. `build_token_stream` in that same file is
//! the only place in the workspace allowed to call `liter_llm::create_client`/
//! `LlmClient::chat_stream` — everywhere else must go through `GuardedLlm::stream`, which
//! redacts first (see that module's doc comment). This test walks every crate's `src/` in
//! the workspace (this crate's own `src/stream.rs` excepted) and fails if any of them
//! calls those functions or constructs `xberg::llm`'s text-completion entry points
//! directly.

use std::path::{Path, PathBuf};

/// Substrings that would mean "this file reaches an LLM without going through
/// `GuardedLlm`." Matched textually, not via `syn`, on purpose — the same tradeoff
/// `hacienda-api/tests/safety.rs`'s `no_from_uri_in_crate_source` makes: a grep-based
/// check that is easy to audit beats a parser-based one that is easy to trust blindly.
const FORBIDDEN: &[&str] = &[
    "liter_llm::create_client",
    ".chat_stream(",
    "xberg::llm::complete_text",
    "xberg::llm::complete_with_json_schema",
    "complete_with_json_schema(",
];

/// The one file allowed to contain the forbidden calls: it's what they name.
const ALLOWED_FILE_SUFFIX: &str = "hacienda-rag/src/stream.rs";

#[test]
fn no_crate_outside_the_guard_calls_an_llm_client_directly() {
    let workspace_root = workspace_root();

    let violations: Vec<String> = walk_rust_files(&workspace_root)
        .filter(|path| !path_is_excluded(path))
        .filter_map(|path| {
            let content = std::fs::read_to_string(&path).ok()?;
            let offending_lines: Vec<String> = content
                .lines()
                .enumerate()
                .filter(|(_, line)| {
                    let trimmed = line.trim_start();
                    !trimmed.starts_with("//") && !trimmed.starts_with('*')
                })
                .filter(|(_, line)| FORBIDDEN.iter().any(|needle| line.contains(needle)))
                .map(|(i, line)| format!("  {}:{}: {}", path.display(), i + 1, line.trim()))
                .collect();
            if offending_lines.is_empty() {
                None
            } else {
                Some(offending_lines.join("\n"))
            }
        })
        .collect();

    assert!(
        violations.is_empty(),
        "found a direct LLM-client call outside GuardedLlm's owning module \
         ({ALLOWED_FILE_SUFFIX}) — route through `hacienda_rag::GuardedLlm::stream` \
         instead, so the call is redacted first (P6):\n\n{}",
        violations.join("\n\n")
    );
}

fn path_is_excluded(path: &Path) -> bool {
    path.to_string_lossy()
        .replace('\\', "/")
        .ends_with(ALLOWED_FILE_SUFFIX)
}

/// This crate lives at `<workspace_root>/crates/hacienda-rag`, so the workspace root is
/// two directories up from `CARGO_MANIFEST_DIR`.
fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("hacienda-rag lives at <workspace_root>/crates/hacienda-rag")
        .to_path_buf()
}

/// Every `src/*.rs` file under every top-level workspace member, skipping `target/` and
/// any vendored/checked-out dependency trees.
fn walk_rust_files(workspace_root: &Path) -> impl Iterator<Item = PathBuf> {
    let mut roots = Vec::new();
    for name in [
        "hacienda",
        "hacienda-core",
        "hacienda-api",
        "hacienda-cli",
        "hacienda-mcp",
    ] {
        roots.push(workspace_root.join(name).join("src"));
    }
    for entry in std::fs::read_dir(workspace_root.join("crates"))
        .into_iter()
        .flatten()
        .flatten()
    {
        let src = entry.path().join("src");
        if src.is_dir() {
            roots.push(src);
        }
    }

    roots
        .into_iter()
        .flat_map(|root| walkdir(&root).collect::<Vec<_>>())
}

fn walkdir(dir: &Path) -> Box<dyn Iterator<Item = PathBuf>> {
    let mut entries = Vec::new();
    if let Ok(read_dir) = std::fs::read_dir(dir) {
        for entry in read_dir.flatten() {
            let path = entry.path();
            if path.is_dir() {
                entries.extend(walkdir(&path));
            } else if path.extension().and_then(|e| e.to_str()) == Some("rs") {
                entries.push(path);
            }
        }
    }
    Box::new(entries.into_iter())
}
