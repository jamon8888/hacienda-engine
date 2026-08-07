# hacienda

> GDPR/DORA/AI Act compliant document intelligence

hacienda extracts text and structure from a wide range of document formats and turns that extraction into a complete, regulator-ready pipeline: personal-data detection, redaction, human review, tamper-evident audit, and generated compliance artefacts — all in one system.

## Features

| Category                      | Capabilities                                                                 |
| ----------------------------- | ---------------------------------------------------------------------------- |
| **Extraction**                | 97+ file formats (documents, images, audio/video, email, archives, code) via xberg |
| **Enrichment**                | NER, classification, captioning, summarization, translation via xberg        |
| **Chunking / Search**         | Chunking, embeddings, reranking, hybrid retrieval via xberg                  |
| **RAG Vector Store**          | In-memory & pgvector backends, streaming LLM answer synthesis                |
| **Plugin architecture**       | Custom OCR, extractor, embedding, reranker, tokenizer, validator, renderer backends |
| **PII Detection**             | 23 built-in regex patterns + GLiNER2 ML model + vertical LoRA adapters       |
| **Redaction**                 | 5 modes: Mask, Hash, Pseudonymize (reversible, AES-256-SIV, key-rotatable), Remove, Custom |
| **Compliance**                | DPIA, Model Card, DORA, AI Act checklists, GDPR Articles                    |
| **Audit**                     | Segmented, hash-chained (blake3) tamper-evident log with durable file/Postgres backends, CSV/JSON export |
| **Review**                    | Human-in-the-loop queue (Approve/Reject/Modify), durable file/Postgres store |
| **Glossary**                  | Entity linking, Markdown/HTML/Wiki link injection                           |
| **Auth & API**                | API keys (Argon2id), capability-based auth (deny-by-default), OpenAPI 3.1   |
| **Presets & Versions**        | Saved pipeline configurations, document versioning & diff                   |
| **Presigned Uploads**         | S3-compatible direct uploads for large documents                            |
| **Usage Metering**            | Per-principal entity/byte counts from audit chain                           |
| **Studio** *(in development)* | Browser workspace for local, zero-egress redaction — in-browser extraction/NER, knowledge-graph export |
| **Vertical NER**              | GLiNER2 base + per-vertical LoRA adapters (business_law taxonomy)           |

## Quick Start

```bash
# Install CLI
cargo install hacienda --features ner-candle

# Scan a document (detect only, no rewrite)
hacienda scan "Contact john@example.com or call +1-555-123-4567"

# Redact PII (mask by default)
hacienda extract "Contact john@example.com" --mode mask

# Pseudonymize (reversible, requires HACIENDA_PSEUDONYM_ACTIVE_KEY)
hacienda extract "Contact john@example.com" --mode pseudonymize

# Full pipeline with compliance, audit, glossary
hacienda extract contract.pdf --pii --compliance --audit-out ./audit --glossary-out ./glossary

# Serve REST API (loopback-only by default)
hacienda serve
```

## Installation

### Rust (facade crate)

```toml
[dependencies]
hacienda = { version = "0.1", features = ["xberg-full", "pii", "compliance", "audit", "review", "glossary"] }
```

### Python (SDK generated from OpenAPI)

```bash
pip install hacienda-sdk
```

### Node.js (SDK generated from OpenAPI)

```bash
npm install @hacienda-engine/sdk
```

### WASM

```bash
npm install @hacienda-engine/hacienda-wasm
```

### Docker

```bash
docker pull ghcr.io/jamon8888/hacienda:latest
docker run --rm -v $(pwd):/data ghcr.io/jamon8888/hacienda:latest \
  hacienda scan "john@example.com"
```

## Architecture

```text
┌─────────────────────────────────────────────────────────────────────────────┐
│                                  hacienda                                    │
│  ┌────────────────────────────────────────────────────────────────────────┐  │
│  │ pub use xberg;                    // 97+ format extraction & enrichment │  │
│  │ pub use hacienda_core::*;         // PII, Compliance, Audit, Review,    │  │
│  │                                    // Glossary, RAG, Auth, Presets,      │  │
│  │                                    // Versions, Uploads, Usage           │  │
│  └────────────────────────────────────────────────────────────────────────┘  │
├─────────────────────────────────────────────────────────────────────────────┤
│                            hacienda-core (private)                           │
│  ┌─────────────┐ ┌─────────────┐ ┌─────────────┐ ┌──────────────────────┐  │
│  │    PII      │ │  Redaction  │ │ Compliance  │ │      Audit           │  │
│  │  Pipeline   │ │   Engine    │ │ Generator   │ │     Chain            │  │
│  └─────────────┘ └─────────────┘ └─────────────┘ └──────────────────────┘  │
│  ┌─────────────┐ ┌─────────────┐ ┌─────────────┐ ┌──────────────────────┐  │
│  │   Review    │ │  Glossary   │ │  Presets &  │ │   Extraction Engine  │  │
│  │   Queue     │ │   Linker    │ │  Versions   │ │   (xberg facade)     │  │
│  └─────────────┘ └─────────────┘ └─────────────┘ └──────────────────────┘  │
│  ┌─────────────┐ ┌─────────────┐ ┌─────────────┐ ┌──────────────────────┐  │
│  │    Jobs     │ │    Auth     │ │    RAG      │ │    Object Store      │  │
│  │   (async)   │ │  (API keys) │ │ (vector DB) │ │   (S3-compatible)    │  │
│  └─────────────┘ └─────────────┘ └─────────────┘ └──────────────────────┘  │
├─────────────────────────────────────────────────────────────────────────────┤
│                         hacienda-rag  |  hacienda-wasm  |  hacienda-studio  │
└─────────────────────────────────────────────────────────────────────────────┘
```

**Built to be extended** — the pipeline is driven entirely through public extension points, so new detectors, formats, and post-processing steps plug in without touching the core:

- `PostProcessor` trait (Late stage, priority 60)
- `NerBackend` trait for custom GLiNER models
- `RedactionConfig::custom_terms` / `custom_patterns`
- Global plugin registries for OCR, extraction, embedding, reranking, tokenizing, validation, and rendering backends
- `RagStore` trait for custom vector backends

## Configuration

```toml
# hacienda.toml
[extraction]
output_format = "json"

[pii]
enabled = true
redaction_profile = "GDPR"  # PCI, HIPAA, GDPR, Default, Custom
model = { enabled = true, model_id = "fastino/GLiNER2-Guardrails-PII-Multi" }
vertical = { id = "business_law", labels = ["ContractParty", "Court", "Statute"] }

[compliance]
enabled = true
model_name = "hacienda-pii-v1"
enabled_reports = ["DPIA", "ModelCard", "DORA", "AIAct", "Checklist"]

[audit]
enabled = true

[review]
enabled = true
auto_assign = false
deadline_hours = 24

[glossary]
enabled = true
link_style = "Markdown"
min_confidence = 0.5

[auth]
enabled = true
resolver = "api_keys"  # or "static" or "jwt"

[jobs]
enabled = true

[presets]
enabled = true

[versions]
enabled = true

[rag]
enabled = true
backend = "memory"  # or "pgvector"

[uploads]
enabled = true
backend = "s3"
```

## API Reference

### Rust

```rust
use hacienda::{HaciendaFacade, HaciendaConfig, ExtractInput};
use hacienda_core::pii::PipelineConfig;

let facade = HaciendaFacade::new(HaciendaConfig {
    extraction: Default::default(),
    pii: Some(PipelineConfig::default()),
    compliance: Some(Default::default()),
    audit: Some(Default::default()),
    review: Some(Default::default()),
    glossary: Some(Default::default()),
    auth: Some(Default::default()),
})?;

let result = facade.process(ExtractInput::from_uri("contract.pdf")).await?;
```

### Python (SDK)

```python
from hacienda_sdk import HaciendaClient

client = HaciendaClient(base_url="http://localhost:8080")
result = client.documents.process_documents(text="Contact john@example.com")
print(result.redacted_text)
```

### TypeScript (SDK)

```typescript
import { HaciendaClient } from "@hacienda-engine/sdk";

const client = new HaciendaClient({ baseUrl: "http://localhost:8080" });
const result = await client.documents.processDocuments({ text: "Contact john@example.com" });
console.log(result.redacted_text);
```

### REST API (44 operations, 14 tags)

```bash
# Health
curl http://localhost:8080/health

# OpenAPI schema
curl http://localhost:8080/openapi.json

# PII scan (detection only)
curl -X POST http://localhost:8080/v1/pii/scan \
  -H "Authorization: Bearer <token>" \
  -H "Content-Type: application/json" \
  -d '{"text": "Contact john@example.com"}'

# PII redact
curl -X POST http://localhost:8080/v1/pii/redact \
  -H "Authorization: Bearer <token>" \
  -H "Content-Type: application/json" \
  -d '{"text": "Contact john@example.com"}'

# Pseudonym reveal
curl -X POST http://localhost:8080/v1/pii/reveal \
  -H "Authorization: Bearer <token>" \
  -H "Content-Type: application/json" \
  -d '{"token": "[EMAIL:k1:MZXW6YTB...]"}'

# Audit chain
curl -X GET http://localhost:8080/v1/audit \
  -H "Authorization: Bearer <token>"

# Review queue
curl -X GET http://localhost:8080/v1/review \
  -H "Authorization: Bearer <token>"

# RAG collections
curl -X POST http://localhost:8080/v1/rag/collections \
  -H "Authorization: Bearer <token>" \
  -H "Content-Type: application/json" \
  -d '{"name":"docs","embedding_dim":384,"distance_metric":"cosine"}'

# Presigned upload
curl -X POST http://localhost:8080/v1/uploads/presign \
  -H "Authorization: Bearer <token>" \
  -H "Content-Type: application/json" \
  -d '{"filename":"large.pdf","content_type":"application/pdf"}'
```

### Capability Model (deny-by-default)

| Capability          | Grants Access To                                    |
| ------------------- | --------------------------------------------------- |
| `DocumentsProcess`  | `/v1/documents*`, `/v1/pii/*`, `/v1/jobs*`, `/v1/rag*`, `/v1/presets*`, `/v1/uploads*`, `/v1/versions*` |
| `AuditRead`         | `/v1/audit*`, `/v1/compliance*`, `/v1/glossary`, `/v1/review` (read), `/v1/usage` |
| `ReviewDecide`      | `POST /v1/review/{id}/decide`                      |
| `PiiReveal`         | `POST /v1/pii/reveal`                              |
| `AuthManage`        | `/v1/auth/keys*`, `/v1/auth/config`                |

## CLI Reference

```bash
# Extract (redacted by default)
hacienda extract <inputs...> [--mode mask|hash|pseudonymize|remove] \
  [--threshold 0.7] [--model-dir ./model] [--lora-dir ./lora] \
  [--format json|text] [--audit-out ./audit] [--vault ./vault] \
  [--glossary-out ./glossary] [--concurrency 4]

# Scan (detection only, never emits document text)
hacienda scan <inputs...> [--threshold 0.7] [--model-dir ./model] \
  [--lora-dir ./lora] [--format json|text] [--glossary-out ./glossary] \
  [--concurrency 4]

# Config inspection
hacienda config show [--format json|text]

# Serve HTTP API
hacienda serve [--bind 127.0.0.1:8787]

# Pseudonym token reversal
hacienda pii reveal <TOKEN> [--format json|text]

# Audit chain verification (flat export from --audit-out)
hacienda audit verify <dir> [--format json|text]
```

### Key CLI Flags

| Flag                          | Description                                           |
| ----------------------------- | ----------------------------------------------------- |
| `--mode`                      | Redaction mode: `mask`, `hash`, `pseudonymize`, `remove` |
| `--threshold`                 | Minimum detection confidence (0.0–1.0)                |
| `--model-dir` / `--lora-dir`  | Local NER model and LoRA adapter directories          |
| `--audit-out`                 | Write audit chain to directory                        |
| `--vault`                     | Export Track I2 vault (documents/, pii-registry.json) |
| `--glossary-out`              | Write entity glossary to directory                    |
| `--no-redact`                 | Emit unredacted text (requires `--i-accept-unredacted-pii`) |
| `--concurrency`               | PII pipeline worker count (default: CPU count)        |

## Development

### Prerequisites

```bash
# One-command setup
task setup

# Or manually:
rustup toolchain install nightly
cargo install cargo-alef cargo-poly task cargo-hack
poly hooks install
```

### Daily Workflow

```bash
task check          # fmt + lint
task test           # Rust tests
task test:all       # All bindings tests
task alef:generate  # Regenerate bindings
task alef:verify    # Verify bindings match source
```

### Running Tests

```bash
# Rust workspace tests
cargo test --workspace

# Live Postgres integration tests
DATABASE_URL=postgres://... cargo test --features postgres -- --ignored

# Python SDK tests
cd sdks/python && uv run pytest

# TypeScript SDK tests
npm run test:unit --workspace=sdks/typescript

# Studio tests
cd apps/hacienda-studio && npm run test:unit && npm run test:e2e
```

### Release

```bash
git tag -s v0.2.0 -m "hacienda 0.2.0"
git push origin v0.2.0
gh workflow run publish.yaml -f tag=v0.2.0
```

## Crate Structure

| Crate                      | Role                                                    |
| -------------------------- | ------------------------------------------------------- |
| `hacienda`                 | Distribution facade re-exporting `xberg` + `hacienda-core` |
| `hacienda-core`            | PII pipeline, redaction, compliance, audit, review, glossary, auth, jobs, presets, versions, RAG |
| `hacienda-api`             | Axum REST API (44 operations, OpenAPI 3.1)             |
| `hacienda-cli`             | CLI binary (`extract`, `scan`, `serve`, `pii`, `audit`, `config`) |
| `crates/hacienda-rag`      | RAG vector store trait + InMemory/PgVector backends     |
| `crates/hacienda-wasm`     | wasm-bindgen entry points for browser PII/audit         |
| `apps/hacienda-studio`     | React 18 + Vite + shadcn/ui + CodeMirror 6 browser app  |
| `sdks/python`              | Python client SDK (`hacienda-sdk` on PyPI)              |
| `sdks/typescript`          | TypeScript client SDK (`@hacienda-engine/sdk` on npm)   |

## Bindings

### REST API clients (real, generated from the live OpenAPI schema)

| Language   | Package                            | Status |
| ---------- | ---------------------------------- | ------ |
| Python     | `sdks/python` (hacienda-sdk on PyPI) | ✅     |
| TypeScript | `sdks/typescript` (@hacienda-engine/sdk on npm) | ✅     |

Generated in this repo (`sdks/`) against `GET /openapi.json` on every CI run — see `sdks/README.md`.
`publish-sdk.yaml` is scaffolded but not activated (needs org-level trusted-publishing setup).

### In-browser (hand-written, narrow scope)

| Language | Package                        | Status |
| -------- | ------------------------------- | ------ |
| WASM     | `crates/hacienda-wasm` (not published) | ✅ (used by Hacienda Studio only) |

### Native FFI bindings (planned, not yet generated)

The table this used to show — Python, Node.js, WASM, Ruby, PHP, Go, Java, C#, Elixir, Dart,
Kotlin/Android, Swift, Zig, C FFI, all "✅" via a `cargo-alef`-generated `packages/` tree — does
not reflect this repo: `packages/` does not exist, `alef.toml` references six Rust source files
that were never written (`hacienda/src/{cli,api,prelude,config}.rs`,
`hacienda-core/src/pii/{profiles,xberg_integration}.rs`), and `.github/workflows/publish.yaml`'s
build/publish matrix has nothing to build. Generating these bindings is unstarted work, not a
`cargo alef generate` away — tracked as a roadmap item, not claimed here until it exists.

## License

Apache-2.0 — see [LICENSE](LICENSE) for details.

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md) for guidelines.

---

[![Built with alef](https://img.shields.io/badge/built%20with-alef%20%D7%90-007ec6)](https://github.com/xberg-io/alef)