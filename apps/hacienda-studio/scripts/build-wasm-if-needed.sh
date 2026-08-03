#!/usr/bin/env bash
# `wasm-pack build` always re-runs its `wasm-opt` optimization pass in full, even when cargo
# itself is fully incremental — on this workspace that pass alone takes ~20 minutes. Because
# predev/pretest:unit/prebuild ran the unguarded build on every single `npm run dev` /
# `test:unit` / `test:e2e` invocation, each of those commands paid the full rebuild even when
# nothing in the crate had changed. This skips the rebuild when the existing pkg/ output is
# already newer than every input that could change it.
set -euo pipefail

STUDIO_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
WASM_CRATE_DIR="$STUDIO_DIR/../../crates/hacienda-wasm"
WORKSPACE_ROOT="$STUDIO_DIR/../.."
OUTPUT="$WASM_CRATE_DIR/pkg/hacienda_wasm_bg.wasm"

if [[ ! -f "$OUTPUT" ]]; then
  echo "[build-wasm-if-needed] No existing pkg/ output — building."
  exec npm run build:wasm --prefix "$STUDIO_DIR"
fi

# Any source file, crate manifest, or the workspace lockfile changing invalidates the build.
newest_input=$(find \
  "$WASM_CRATE_DIR/src" \
  "$WASM_CRATE_DIR/Cargo.toml" \
  "$WORKSPACE_ROOT/Cargo.toml" \
  "$WORKSPACE_ROOT/Cargo.lock" \
  -type f -newer "$OUTPUT" 2>/dev/null | head -1)

if [[ -n "$newest_input" ]]; then
  echo "[build-wasm-if-needed] $newest_input changed since last build — rebuilding."
  exec npm run build:wasm --prefix "$STUDIO_DIR"
fi

echo "[build-wasm-if-needed] pkg/ output is up to date — skipping wasm-pack build."
