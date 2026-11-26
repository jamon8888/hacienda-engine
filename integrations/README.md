# hacienda integrations

First-party integrations that connect **hacienda-engine's redacting, audited document
pipeline** to popular AI and data frameworks. Modeled on
[xberg's own `integrations/` folder](https://github.com/xberg-io/xberg/tree/main/integrations)
(hacienda's `xberg` dependency — see the root `Cargo.toml`) — same layout, same
per-language conventions — but every **implemented** adapter here calls
`hacienda-api` (`POST /v1/documents`, `/v1/pii/*`, `/v1/rag/*`) instead of raw xberg
extraction. ("Implemented" vs. "Planned" is tracked in the status table below —
planned packages are manifests/READMEs only, no adapter code yet.)

## Why this is not just "xberg's integrations, renamed"

xberg's loaders answer "how do I get documents into my RAG pipeline." These adapters
send inputs through Hacienda's configured document-processing API before returning
content to the host framework. `POST /v1/documents` returns processed content, detected
entities, and optional document/version identifiers. The current `DocumentResult` does
**not** include an `audit_chain_tip` or rich extraction metadata; see
[`LIMITATIONS.md`](./LIMITATIONS.md). Implemented adapters preserve the fields they
receive as framework metadata. Traceability is limited to the identifiers and metadata
actually returned by the API, not a per-batch audit-tip guarantee.

| Integration | Path | Status |
|-------------|------|--------|
| LangChain (Python) | `python/langchain` | **Implemented** — `HaciendaLoader`, `HaciendaRetriever`, `push_documents` |
| LangChain (Node) | `node/langchain` | **Implemented** — `HaciendaLoader`, `HaciendaRetriever`, `pushDocuments` |
| LlamaIndex (reader) | `python/llama-index` | Planned |
| CrewAI | `python/crewai` | Planned |
| txtai | `python/txtai` | Planned |
| n8n | `node/n8n-nodes-hacienda` | Planned |
| Spring AI (Java) | `java/spring-ai` | Planned |

"Planned" packages have real package manifests (installable/buildable placeholders,
same as xberg's own `integrations/scripts/publish-placeholders.sh` pattern for
not-yet-released packages) and a README describing the intended shape, but no adapter
code yet — see each one's README for what porting it from xberg's equivalent would
involve.

**Before assuming feature parity with xberg's loaders, read
[`LIMITATIONS.md`](./LIMITATIONS.md)** — it tracks every confirmed gap (server-side
limitations an integration package can't close, and deliberate scope decisions) in one
place, so it doesn't need restating per package.

## Layout

- **Python** packages are standalone [uv](https://docs.astral.sh/uv/) projects, each
  owning its own lockfile — not uv-workspace members, to avoid cross-framework resolver
  conflicts (same reasoning as xberg's `integrations/README.md`). Work on one with
  `cd integrations/python/<name> && uv sync --all-extras`.
- **Node** packages are root npm workspace members (`integrations/node/*` in the root
  `package.json`'s `workspaces` array) — there is no per-package lockfile, only the
  root `package-lock.json`. Install and build from the repo root:
  `npm ci && npm run build --workspace=@hacienda-engine/langchain-hacienda`. Planned/
  scaffold Node packages (`node/n8n-nodes-hacienda`) deliberately have no `build`
  script yet — see the status table above — so `turbo run build` at the repo root
  skips them rather than failing.
- **Java** (`java/spring-ai`) is a Maven project.

## Dependencies

Every implemented adapter depends on this repo's own client SDK for its language
(`sdks/python`'s `hacienda-sdk`, `sdks/typescript`'s `@hacienda-engine/sdk`) rather than
talking to `hacienda-api` directly — same reasoning `sdks/README.md` gives for those
SDKs existing at all: one generated, retrying, typed client per language, not one per
integration.

## Versioning

Mirrors `sdks/VERSION`'s convention: an integration's own package version is independent
of `hacienda-engine`'s, but its `hacienda-sdk` / `@hacienda-engine/sdk` dependency pin
should track the SDK version it was built and tested against.

## Docs

Each package's own README is the primary reference. `python/langchain/README.md` and
`node/langchain/README.md` are the fullest examples — read one of those first if you're
porting the next integration from xberg's equivalent.
