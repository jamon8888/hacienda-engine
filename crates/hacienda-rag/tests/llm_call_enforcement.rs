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
///
/// Known gaps in this approach, accepted rather than hidden: `use liter_llm::create_client;`
/// followed by a bare `create_client(...)` call, or an `as`-aliased import, would not match
/// the fully-qualified needles — `"create_client("` below closes the common case, but an
/// aliased import (`use ... as make_client;`) still evades every needle here. And the scan
/// (`walk_rust_files`) only covers each crate's `src/`, not `tests/`/`benches/`/`build.rs` —
/// a direct LLM call in a test file is invisible to this check. Both gaps make this a
/// second line of defense, not a substitute for the actual guarantee `GuardedLlm` provides
/// structurally within `hacienda-rag` itself (nothing outside this crate can name
/// `liter_llm` without adding it as a direct dependency, which `cargo-deny`/code review
/// would flag as its own signal).
const FORBIDDEN: &[&str] = &[
    "liter_llm::create_client",
    "create_client(",
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
///
/// This is an enforcement test, so a scan root that silently stops existing must fail the
/// build, not just scan fewer files: a renamed or removed top-level crate used to leave the
/// list below scanning nothing for it and the test would still pass, which is worse than
/// not having the test at all — a reader trusts a green enforcement test. Every hardcoded
/// root's `src/` is asserted to exist, and `crates/` itself must exist and be readable
/// (`expect`, not the old `.into_iter().flatten().flatten()`, which silently produced zero
/// roots for a missing or unreadable directory).
fn walk_rust_files(workspace_root: &Path) -> impl Iterator<Item = PathBuf> {
    let mut roots = Vec::new();
    for name in [
        "hacienda",
        "hacienda-core",
        "hacienda-api",
        "hacienda-cli",
        "hacienda-mcp",
    ] {
        let src = workspace_root.join(name).join("src");
        assert!(
            src.is_dir(),
            "workspace member '{name}' has no src/ at {} — update this list, otherwise the \
             LLM-call enforcement check silently stops covering it",
            src.display()
        );
        roots.push(src);
    }

    let crates_dir = workspace_root.join("crates");
    let crate_entries =
        std::fs::read_dir(&crates_dir).expect("workspace has a crates/ directory to scan");
    for entry in crate_entries.flatten() {
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
