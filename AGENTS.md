# Hacienda Engine — agent guide

Use this file as the shared project map. Read the relevant component documentation before changing a subsystem; do not treat the overview below as a substitute for its design or API contract.

## Product and current shape

Hacienda is a polyglot document-intelligence workspace focused on PII detection and redaction, auditability, review, compliance artefacts, retrieval, and local/client applications. The implementation is split across a Rust Cargo workspace, web applications, language SDKs, integrations, and tooling. The Studio is in development; verify feature status in code/docs before describing it as shipped.

The root `Cargo.toml` currently lists 13 Rust workspace members. The root `package.json` manages JavaScript workspaces with pnpm and Turbo. `Taskfile.yml` is the project task catalog. Treat manifests and command `--help` output as authoritative when docs differ.

## Repository map

| Path | Responsibility |
|---|---|
| `hacienda/` | Public Rust facade and distribution crate; re-exports the upstream `xberg` surface with Hacienda functionality. |
| `hacienda-core/` | Core domain: PII pipeline, redaction, compliance, audit, human review, glossary, auth, jobs, presets, versions, uploads, and usage. |
| `hacienda-api/` | Axum REST API and OpenAPI surface over the facade/core. |
| `hacienda-cli/` | `hacienda` command-line application. |
| `hacienda-mcp/` | MCP server exposing Hacienda operations to agents. |
| `crates/hacienda-rag/` | Retrieval/vector-store contracts and backends. |
| `crates/hacienda-backend/` | Token-only Xberg projection and profile-scoped RAG integration. |
| `crates/hacienda-email/` | Email processing plugin and email ecosystem integration. |
| `crates/hacienda-installation/`, `hacienda-client-core/` | Installation/profile resolution and client trust-boundary contracts. |
| `crates/hacienda-model-assets/` | Pinned model digests and model-asset verification. |
| `crates/hacienda-wasm/` | wasm-bindgen entry points for browser-side PII/redaction functionality. |
| `crates/hacienda-legal/` | Optional legal research providers and source-scoped retrieval. |
| `apps/hacienda-studio/` | React/Vite browser workspace. |
| `apps/hacienda-secrecy-client/` | Tauri desktop client. |
| `apps/harness-chat-adapter/` | Standalone DeepSeek Harness RPC adapter. |
| `sdks/` | Generated REST clients and SDKs; see `sdks/README.md`. |
| `integrations/` | Framework and product integrations; see `integrations/README.md`. |
| `docs/` | User, API, configuration, architecture, research, and implementation documents. |
| `superpowers/specs/`, `superpowers/plans/` | Design and implementation records. Check status/date and current code before relying on a plan. |
| `vps-harness/` | User-level agent setup and gatekeeper; see its README and INSTALL guide. |
| `scripts/`, `Taskfile.yml`, `.github/workflows/` | Development, CI, generation, and release automation. |

For the current dependency graph, routes, import hotspots, and latest repository snapshot, read generated `docs/CODEMAP.md` when present. Regenerate with `codemap generate --root .`; it is ignored by Git.

## Where to look first

- Configuration behavior or a new config field: `docs/configuration.md`, then `hacienda-core/src/config.rs` and the owning config type. Config structs use strict deserialization; unknown fields should fail rather than silently pass.
- REST behavior, auth, or API evidence: `hacienda-api/README.md`, `hacienda-api/`, and the OpenAPI source/generation workflow.
- CLI behavior: `hacienda-cli/README.md` if present, then `hacienda-cli/src/` and `hacienda --help`.
- Architecture/product decisions: `docs/architecture/README.md` first. It explains reading order and records superseded conclusions.
- Subsystem design: the nearest crate/module `README.md`, `DESIGN.md`, or module docs; then its source and tests.
- SDK contracts and generated files: `sdks/README.md`; update the source schema/generator, not generated output alone.
- Integrations: `integrations/README.md` and the specific integration README.
- CI or test behavior: `scripts/ci/README.md`, the relevant workflow, then `Taskfile.yml`.

## Context tools

- **Basemind** indexes code symbols/references, text, documents, Git history, and memory. Its code-file count is not every file: ignored/build output, binary or unsupported files, and configured exclusions are skipped; documents use a separate search tier. Use it before broad shell searches or opening files for discovery. Check `basemind status --json` when results are empty/stale; use `basemind rescan <path>` after focused edits and a full `basemind rescan` after a large out-of-band change.
- **Context7** is for current upstream library/framework/SDK/API documentation. Resolve the package/library ID before querying. Use it for version-sensitive external API behavior; Basemind answers what this repository contains.
- Use ordinary file reads and shell search for details the context tools do not expose, generated/unindexed files, and final verification.

Root `basemind.toml` keeps document extraction/indexing enabled but disables local document vectors; document keyword search remains available. It excludes the large generated GLiNER tokenizer JSON. Keep derived Basemind indexes and machine-specific state outside Git. Enable local vectors only when their additional CPU and storage cost is wanted.

## Security and data handling

This product handles personal and potentially regulated document content. Keep secrets, keys, and real customer/PII examples out of source, logs, fixtures, commits, and tool prompts. Prefer synthetic test values. Preserve strict config validation, capability checks, tenant/profile boundaries, redaction defaults, and audit verification when touching those paths.

Redaction defaults to `mask`; unredacted output requires the explicit CLI acknowledgement flag. `hacienda serve` is loopback-only unless auth is enabled. Read `docs/configuration.md` before changing bind/auth behavior or pseudonymisation keys. Treat audit records as evidence with their documented chain semantics; do not replace them with ordinary application logs.

Changes involving reversible pseudonymisation must preserve key IDs, active-key selection, rotation behavior, and the documented token contract. Changes involving persistence must preserve tenant/profile scope and error paths that prevent silent data loss.

## Development workflow

Use the narrowest command that covers the changed component, then the project gate when practical:

```sh
task check:rust                  # fast Rust type-check, including targets
task test                        # hacienda + hacienda-core tests
cargo test -p <crate> <filter>   # focused Rust regression
task check                       # formatting and lint checks
task verify                      # formatting, full lint, and Rust tests
task test:all                    # Rust plus language-binding tests
task test:e2e                    # binding E2E tests
pnpm check                       # TypeScript workspace checks
pnpm test:unit                   # TypeScript unit tests
```

The repository uses nightly Rust for its full tool setup. `task setup` installs toolchains and project tools. For TypeScript changes, run the root pnpm check and unit-test scripts or the narrower scripts in the affected workspace. Python SDK tests run from `sdks/python` with `uv run pytest`. Live Postgres tests are ignored by default and require `DATABASE_URL` plus the feature documented by the test.

Before changing generated bindings, inspect `alef.toml` and `Taskfile.yml`; use `task alef:verify` to check generated output. WASM/Studio changes may require `task build:wasm` before Studio dev/build. Do not claim native FFI bindings exist: the README marks that work as planned.

Use conventional commit subjects (`feat:`, `fix:`, `docs:`, `test:`, `chore:`). Keep changes within the owning crate/app and update the adjacent docs when a public contract changes.

## Agent harness

The repo's `scripts/codemap.sh` generates the ignored `docs/CODEMAP.md`. `scripts/setup-codemap-user.sh` manages the user-level Codemap installation and Orca refresh automation; read `docs/code-map-harness.md` before changing or reinstalling that automation. The shared user-level harness is under `vps-harness/` and installed as `agent-harness`. Use `task verify` and the relevant root/package scripts as the Hacienda checks; the harness gatekeeper is a generic runner, not a replacement for those project commands. When changing the harness, consult its README and run its offline unit suite.
