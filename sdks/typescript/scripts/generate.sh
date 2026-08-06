#!/usr/bin/env bash
#
# Regenerate src/_generated/api.d.ts (types only — openapi-fetch needs no
# per-operation client code, unlike openapi-python-client) from the live
# OpenAPI document. Gitignored output — run before test/build, never committed.
set -euo pipefail

PKG_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
REPO_ROOT="$(cd "${PKG_DIR}/../.." && pwd)"
OUT_DIR="${PKG_DIR}/src/_generated"

SPEC_FILE="$(mktemp)"
trap 'rm -f "${SPEC_FILE}"' EXIT
bash "${REPO_ROOT}/sdks/scripts/fetch-openapi.sh" > "${SPEC_FILE}"

mkdir -p "${OUT_DIR}"
cd "${PKG_DIR}"
npx --no-install openapi-typescript "${SPEC_FILE}" -o "${OUT_DIR}/api.d.ts"
