# Hacienda Engine

Hacienda processes documents and protects personal data across extraction, redaction, review, audit, retrieval, and AI integrations. The project combines a Rust engine and API with a CLI, MCP server, browser and desktop applications, SDKs, and framework integrations.

This README is the quick entry point. The [feature and status catalog](docs/features-and-roadmap.md) is the detailed inventory of what is implemented, what needs configuration, known limits, and proposed improvements. Historical design documents under `docs/architecture/` and `superpowers/` are plans or point-in-time analysis; they do not override the current code or that catalog.

## What Hacienda provides

- **Document extraction:** Xberg-based extraction and MIME detection for the formats enabled by the build. Use `hacienda formats` to inspect the formats in a particular binary.
- **PII processing:** built-in regex detection, checksummed validators for selected identifiers, optional neural NER, and mask, hash, pseudonymize, or remove redaction modes.
- **Audit and review:** hash-linked audit records, verification/export, and a human review queue with assignment and decisions.
- **Compliance reports:** DPIA, AI Act model-card/checklist, DORA incident report, and combined reports from configured inputs.
- **RAG:** collections, chunking, embeddings, retrieval, graph queries, and streaming answers, with backend-specific capabilities.
- **Interfaces:** Rust crates, REST API, CLI, MCP server, browser Studio, Tauri Secrecy Client, Python/TypeScript REST SDKs, and implemented LangChain adapters.

## Quick start

Install from this repository:

```sh
cargo install --git https://github.com/jamon8888/hacienda-engine --package hacienda-cli
```

Scan text without rewriting it:

```sh
hacienda scan ./document.pdf
```

Extract and redact text (mask is the default mode):

```sh
hacienda extract ./document.pdf --mode mask
```

Run the API locally (loopback by default):

```sh
hacienda serve
curl http://127.0.0.1:8787/health
```

For pseudonymization, configure the active key and key material first. For network-accessible API binds, enable authentication and replace example credentials. See [configuration](docs/configuration.md).

## Workspace map

| Component | Purpose |
| --- | --- |
| `hacienda-core/` | PII, redaction, auth, audit, review, compliance, glossary, jobs, presets, versions, uploads, and usage |
| `hacienda/` | Public Rust facade and Xberg re-exports |
| `hacienda-api/` | Axum REST API and generated OpenAPI contract |
| `hacienda-cli/` | `hacienda` command-line application |
| `hacienda-mcp/` | Model Context Protocol server |
| `crates/hacienda-rag/` | RAG contracts and memory, LanceDB, and PostgreSQL/pgvector backends |
| `crates/hacienda-email/`, `crates/hacienda-legal/` | Email processing and legal research providers |
| `crates/hacienda-wasm/` | Browser-side Rust/WASM PII and redaction engine |
| `apps/hacienda-studio/` | Browser document workspace; in development |
| `apps/hacienda-secrecy-client/` | Tauri desktop client for guarded Claude/Codex workflows |
| `sdks/`, `integrations/` | Generated REST SDKs and framework adapters |
| `vps-harness/`, `scripts/`, `.agents/` | Agent harness, codemap and repository automation |

## Documentation

- [Documentation index](docs/README.md)
- [Feature inventory, operational limits, and improvement proposals](docs/features-and-roadmap.md)
- [Configuration and security-sensitive settings](docs/configuration.md)
- [REST API README and OpenAPI](hacienda-api/README.md), or `GET /openapi.json` on a running server
- [Python and TypeScript SDKs](sdks/README.md)
- [Framework integrations and their status](integrations/README.md) and [confirmed integration limits](integrations/LIMITATIONS.md)
- [Agent guidance](AGENTS.md) and [Claude Code setup](CLAUDE.md)
- [Agent harness and live codemap](docs/code-map-harness.md)
- [Architecture document index](docs/architecture/README.md)

## Development

The repository uses Rust, Node/pnpm workspaces, Python/uv packages, and a Tauri app. The task catalog in `Taskfile.yml` is authoritative for commands.

```sh
task setup       # install project toolchain and tools
task check       # formatting and lint checks
task test        # core Rust tests
task verify      # repository verification gate
pnpm check       # JavaScript/TypeScript checks
```

Run the checks for the component you changed. Postgres integration tests require a configured database; model-backed NER tests and applications may require separately installed model assets. Read [AGENTS.md](AGENTS.md) before making repository changes.

## Project status

Capabilities vary by compiled feature, configured storage backend, authentication setup, and installed model assets. The Studio remains in development. Some integration directories are placeholders; [integrations/README.md](integrations/README.md) identifies which adapters have code. Pull request status is dynamic and belongs on GitHub rather than in this README.

## License

See [LICENSE](LICENSE).
