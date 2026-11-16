#!/usr/bin/env bash
# Smoke test for the real GLiNER2 + Xberg model path (Mission P2).
#
# Runs the existing, `#[ignore]`d GLiNER2 provider path test against a local
# pinned GLiNER2 checkpoint, plus documents the Xberg embedding cache
# requirement for the real embedding/hybrid-search path.
#
# No model weights are downloaded here. All tests that need weights require
# `HACIENDA_NER_MODEL_DIR` to point at an already-installed checkpoint
# (config.json, model.safetensors, tokenizer.json, encoder_config/config.json,
# tokenizer_config.json) and are ignored otherwise — fail-closed, never mocked.
#
# Usage:
#   HACIENDA_NER_MODEL_DIR=/path/to/gliner2-pii ./scripts/smoke-real-model.sh
#
# Without HACIENDA_NER_MODEL_DIR the ignored tests are skipped and the script
# exits 0 after printing the embedding-cache setup note.
set -Eeuo pipefail

ROOT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

TAURI_DIR="apps/hacienda-secrecy-client/src-tauri"
TAURI_MANIFEST="$TAURI_DIR/Cargo.toml"

echo "[smoke-real-model] Repository root: $ROOT_DIR"

if [[ -z "${HACIENDA_NER_MODEL_DIR:-}" ]]; then
    echo "[smoke-real-model] HACIENDA_NER_MODEL_DIR is not set."
    echo "  -> Skipping weight-dependent GLiNER2 smoke test (it is #[ignore]d)."
    echo "  -> To run the real GLiNER2 smoke:"
    echo "      HACIENDA_NER_MODEL_DIR=/path/to/gliner2-pii $0"
    echo ""
    echo "Embedding cache (Xberg multilingual preset) is a separate prerequisite:"
    echo "  The client installs/warms the Xberg embedding model under"
    echo "  <app-data>/models/xberg-embeddings (see install_local_rag_model /"
    echo "  install_embedding_model in $TAURI_DIR/src/main.rs)."
    echo "  No cache is required to compile; only to execute the real embedding/"
    echo "  hybrid-search path. Warm it once per machine via the Tauri command"
    echo "  install-local-rag-model after the app bundles ONNX Runtime."
    echo ""
    echo "This script validates the GLiNER2 NER smoke when weights are provided;"
    echo "it does not run the embedding/hybrid path without a warmed cache."
    exit 0
fi

MODEL_DIR="$HACIENDA_NER_MODEL_DIR"
if [[ ! -d "$MODEL_DIR" ]]; then
    echo "[smoke-real-model] ERROR: HACIENDA_NER_MODEL_DIR='$MODEL_DIR' is not a directory" >&2
    exit 2
fi
for asset in config.json model.safetensors tokenizer.json tokenizer_config.json encoder_config/config.json; do
    if [[ ! -f "$MODEL_DIR/$asset" ]]; then
        echo "[smoke-real-model] ERROR: missing model asset '$MODEL_DIR/$asset'" >&2
        exit 2
    fi
done

echo "[smoke-real-model] Running real GLiNER2 provider-path test (not mocked, not WASM)"
echo "  model dir: $MODEL_DIR"

# Run the existing ignored test that exercises the real Candle GLiNER2 backend
# end-to-end through HaciendaLocalRedactor + ProviderCommandState::stream.
# The Tauri app is its own Cargo workspace (binary package hacienda-secrecy-client);
# run from its manifest with --bin.
# NOTE: compiling the Tauri app requires ONNX Runtime prebuilt binaries for
# `ort-sys` (xberg `ort-dynamic` feature). If they are unavailable for this
# target, cargo will fail at the ort-sys build script — that is a packaging
# prerequisite, not a test failure. Install the ONNX Runtime runtime lib for
# the target first (see ONNXRUNTIME_LIBRARY_FILE / ONNXRUNTIME_RESOURCE_TARGET
# in $TAURI_DIR/src/main.rs).
(cd "$TAURI_DIR" && \
    HACIENDA_NER_MODEL_DIR="$MODEL_DIR" \
    cargo test --bin hacienda-secrecy-client \
        pinned_gliner2_model_redacts_claude_codex_requests_and_responses \
        -- --ignored --exact 2>&1) \
    || { echo "[smoke-real-model] GLiNER2 smoke test FAILED (see above; ort-sys prebuilt binaries may be missing for this target)" >&2; exit 1; }

echo "[smoke-real-model] GLiNER2 smoke test passed (real model, no mock, no WASM)."
echo ""
echo "This smoke covers the GLiNER2 NER path only."
echo "The Xberg embedding/hybrid-search path additionally requires the embedding cache:"
echo "  1. Set HACIENDA_NER_MODEL_DIR (done above) and run this script once to validate weights."
echo "  2. Inside the Tauri app, call install-local-rag-model to warm"
echo "     <app-data>/models/xberg-embeddings (real Xberg multilingual preset)."
echo "  3. Only then is ingestion + hybrid search (RetrieveMode::Hybrid) exercised."
