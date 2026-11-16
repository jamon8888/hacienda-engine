#!/usr/bin/env bash
# Verification gate for the secrecy-client foundation and its token-only
# editor/RAG boundaries. Keep this script explicit: a new touched crate or
# feature should be added here instead of silently falling through to a generic
# workspace build.
set -Eeuo pipefail

ROOT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../../.." && pwd)"
cd "$ROOT_DIR"

failures=0

run_gate() {
    printf '\n[secrecy-client-gate] %q' "$1"
    shift
    printf ' %q' "$@"
    printf '\n'
    if "$@"; then
        return 0
    else
        status=$?
    fi
    failures=$((failures + 1))
    printf '[secrecy-client-gate] FAILED (status %d)\n' "$status" >&2
    return 0
}

# Keep every failure visible in CI. In particular, a clippy diagnostic must be
# emitted by clippy itself rather than being hidden behind a later generic CI
# step; the final status is returned after all diagnostics have run.
run_gate format cargo fmt --all -- --check

# The Tauri app has an independent Cargo workspace, so the repository-wide
# formatter does not include it. Keep its manifest explicit for the same
# reason as the focused workspace checks below.
run_gate secrecy-client-format \
    cargo fmt --manifest-path apps/hacienda-secrecy-client/src-tauri/Cargo.toml -- --check

run_gate secrecy-client-npm-ci \
    npm ci --prefix apps/hacienda-secrecy-client
run_gate secrecy-client-check \
    npm run check --prefix apps/hacienda-secrecy-client

# Tauri resolves bundle resources in its build script even for `cargo clippy`.
# Stage the checksum-verified native runtime before the client Cargo gate.
run_gate secrecy-client-onnxruntime \
    npm run prepare:onnxruntime --prefix apps/hacienda-secrecy-client

run_gate client-tests cargo test -p hacienda-client-core
run_gate rag-tests cargo test -p hacienda-rag --features lancedb
run_gate backend-tests cargo test -p hacienda-backend --all-targets

run_gate client-clippy cargo clippy -p hacienda-client-core --all-targets -- -D warnings
run_gate core-clippy cargo clippy -p hacienda-core --all-targets -- -D warnings
run_gate rag-clippy cargo clippy -p hacienda-rag --features lancedb --all-targets -- -D warnings
run_gate backend-clippy cargo clippy -p hacienda-backend --all-targets -- -D warnings

run_gate secrecy-client-clippy \
    cargo clippy --manifest-path apps/hacienda-secrecy-client/src-tauri/Cargo.toml \
        --all-targets -- -D warnings

# This module test serializes the complete editor envelope and asserts that
# fixture plaintext from body, title, subject, filename, attachment, and keyword
# fields is absent while opaque token references remain present.
run_gate token-only-negative-tests cargo test -p hacienda-core --lib editor_projection

# This is intentionally the one workspace-wide attempt. The earlier clippy
# commands are the focused diagnostics for the touched crates/features.
run_gate workspace-clippy \
    cargo clippy --workspace --all-targets --all-features -- -D warnings

if (( failures )); then
    printf '[secrecy-client-gate] %d gate(s) failed\n' "$failures" >&2
    exit 1
fi

echo "[secrecy-client-gate] all checks passed"
