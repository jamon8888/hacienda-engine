# hacienda SDKs

Official client SDKs for the [hacienda-engine](../README.md) API, generated from its
OpenAPI 3.1 schema (`GET /openapi.json`).

## Layout

```text
sdks/
  python/       PyPI distribution `hacienda-sdk` (httpx-based, sync + async)
  typescript/   npm workspace member `@hacienda-engine/sdk` (openapi-fetch, ESM-only)
  scripts/
    fetch-openapi.sh    starts hacienda-cli, fetches /openapi.json, prints it, shuts down
    sync-versions.py    propagates VERSION into every package manifest
  VERSION       single source of truth for the SDK version across both packages
```

There is no vendored spec file. Both packages regenerate their client code from a
live `hacienda serve` instance built from the same checkout — the spec they generate
against is always the one the commit under test actually serves, not a snapshot that
can drift from it.

## Why this lives in `hacienda-engine`, not a separate repo

`xberg-sdks` (the SDK repo for `xberg`, the upstream crate hacienda depends on) is a
separate repo from its API server, with a cross-repo `spec-sync` workflow to keep a
vendored spec current. hacienda-engine's API and SDKs live in one repo instead:
one fewer moving part (no cross-repo token, no vendored-spec drift), at the cost of
the SDK sharing a release cadence with the engine unless split out later.

## Development

```sh
# Fetch the current OpenAPI document (also called internally by each package's
# own `generate` step):
bash sdks/scripts/fetch-openapi.sh > /tmp/openapi.json

# Python
cd sdks/python && uv sync && bash scripts/generate.sh
uv run pytest

# TypeScript (from the repo root, npm workspace)
npm run generate --workspace=sdks/typescript
npm run test:unit --workspace=sdks/typescript
```

Bump the version with `sdks/scripts/sync-versions.py` after editing `sdks/VERSION`.

## Target

Both clients expose `target: "cloud"` today — they speak HTTPS to `hacienda-api`. A
future `target: "device"` (embedded Cactus runtime, on-device extraction/scan, no
HTTP) is Phase 15 of the platform-parity plan and not implemented yet; the `target`
literal type in both clients is already shaped as a union of one so that phase can
extend it without changing the public API shape.
