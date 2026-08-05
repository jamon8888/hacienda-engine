#!/usr/bin/env bash
#
# Build hacienda-cli, start `hacienda serve` on a loopback port with auth disabled,
# fetch /openapi.json, print it to stdout, and shut the server down.
#
# Used by both sdks/python and sdks/typescript's `generate` steps (dev and CI) so
# there is exactly one way to get the current OpenAPI document — never a vendored
# spec file that can drift from the code it describes.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
PORT="${HACIENDA_OPENAPI_FETCH_PORT:-8787}"
BIND="127.0.0.1:${PORT}"

cd "${REPO_ROOT}"
cargo build -p hacienda-cli >&2

./target/debug/hacienda serve --bind "${BIND}" >&2 &
SERVER_PID=$!
trap 'kill "${SERVER_PID}" 2>/dev/null || true; wait "${SERVER_PID}" 2>/dev/null || true' EXIT

for _ in $(seq 1 50); do
    if curl -s -o /dev/null "http://${BIND}/health"; then
        break
    fi
    sleep 0.2
done

curl -sf "http://${BIND}/openapi.json"
