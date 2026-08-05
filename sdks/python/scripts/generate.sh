#!/usr/bin/env bash
#
# Regenerate src/hacienda_sdk/_generated/ from the live OpenAPI document. Gitignored
# output — run before test/build, never committed. Mirrors xberg-sdks' own
# generate-then-move pattern (openapi-python-client only accepts an output
# directory it creates itself, so generate into a throwaway dir and move the
# contents up).
set -euo pipefail

PKG_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
REPO_ROOT="$(cd "${PKG_DIR}/../.." && pwd)"
OUT_DIR="${PKG_DIR}/src/hacienda_sdk/_generated"

SPEC_FILE="$(mktemp)"
trap 'rm -f "${SPEC_FILE}"' EXIT
bash "${REPO_ROOT}/sdks/scripts/fetch-openapi.sh" > "${SPEC_FILE}"

rm -rf "${OUT_DIR}"
mkdir -p "${OUT_DIR}"

cd "${PKG_DIR}"
uv run openapi-python-client generate \
    --path "${SPEC_FILE}" \
    --config openapi-python-client.yaml \
    --output-path "${OUT_DIR}/_pkg" \
    --overwrite \
    --meta none

mv "${OUT_DIR}/_pkg"/* "${OUT_DIR}/"
rmdir "${OUT_DIR}/_pkg"
touch "${OUT_DIR}/__init__.py"
