# hacienda integrations

First-party integrations that connect **hacienda-engine's redacting, audited document
pipeline** to popular AI and data frameworks. Modeled on
[xberg's own `integrations/` folder](https://github.com/xberg-io/xberg/tree/main/integrations)
(hacienda's `xberg` dependency — see the root `Cargo.toml`) — same layout, same
per-language conventions — but every adapter here calls `hacienda-api`
(`POST /v1/documents`, `/v1/pii/*`, `/v1/rag/*`) instead of raw xberg extraction.

## Why this is not just "xberg's integrations, renamed"

xberg's loaders answer "how do I get documents into my RAG pipeline." These answer
"how do I get documents into my RAG pipeline **without raw PII ever reaching my vector
store or LLM context, with a cryptographically verifiable audit trail of what was
redacted and when**." `POST /v1/documents` always returns already-redacted content
(per the caller's configured PII pipeline — see `hacienda-api/src/handlers/documents.rs`),
alongside the detected `entities` (category, span, confidence — never raw text unless
`pii:reveal` is granted) and the batch's `audit_chain_tip`. Every adapter in this folder
surfaces that as first-class metadata on whatever object its host framework uses
(LangChain `Document.metadata`, etc.) rather than discarding it, so a chunk sitting in a
vector store can always be traced back to the audited extraction that produced it.

| Integration | Path | Status |
|-------------|------|--------|
| LangChain (Python) | `python/langchain` | **Implemented** — `HaciendaLoader` |
| LangChain (Node) | `node/langchain` | **Implemented** — `HaciendaLoader` |
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

## Layout

- **Python** packages are standalone [uv](https://docs.astral.sh/uv/) projects, each
  owning its own lockfile — not uv-workspace members, to avoid cross-framework resolver
  conflicts (same reasoning as xberg's `integrations/README.md`). Work on one with
  `cd integrations/python/<name> && uv sync --all-extras`.
- **Node** packages are standalone TypeScript packages (`npm ci && npm run build`).
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
