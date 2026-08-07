# hacienda

> GDPR/DORA/AI Act compliant document intelligence

hacienda extracts text and structure from a wide range of document formats and turns that extraction into a complete, regulator-ready pipeline: personal-data detection, redaction, human review, tamper-evident audit, and generated compliance artefacts — all in one system.

## Features

| Category                      | Capabilities                                             |
| ------------------------------ | --------------------------------------------------------- |
| **Extraction**                | 97+ file formats (documents, images, audio/video, email, archives, code) |
| **Enrichment**                | NER, classification, captioning, summarization, translation |
| **Chunking / Search**         | Chunking, embeddings, reranking                          |
| **Plugin architecture**       | Custom OCR, extractor, embedding, reranker, tokenizer, validator, and renderer backends |
| **PII Detection**             | 42 regex patterns + GLiNER2 ML model (42 entity types)   |
| **Redaction**                 | 5 modes: Mask, Hash, Pseudonymize (reversible, AES-256-SIV, key-rotatable), Remove, Custom |
| **Compliance**                | DPIA, Model Card, DORA, AI Act checklists, GDPR Articles |
| **Audit**                     | Segmented, hash-chained (blake3) tamper-evident log with durable file backend, CSV/JSON export |
| **Review**                    | Human-in-the-loop queue (Approve/Reject/Modify), durable store |
| **Glossary**                  | Entity linking, Markdown/HTML/Wiki link injection        |
| **Studio** *(in development)* | Browser workspace for local, zero-egress redaction — in-browser extraction/NER, knowledge-graph export |

## Quick Start

```bash
# Install
cargo install hacienda --features dev

# Scan a document
hacienda pii scan "Contact john@example.com or call +1-555-123-4567"

# Redact PII
hacienda pii redact "Contact john@example.com"

# Full pipeline
hacienda process contract.pdf --pii --compliance --glossary
```

## Installation

### Rust

```toml
[dependencies]
hacienda = { version = "0.1", features = ["dev"] }
```

### Python

```bash
pip install hacienda
```

### Node.js

```bash
npm install @hacienda/hacienda
```

### Docker

```bash
docker pull ghcr.io/jamon8888/hacienda:latest
docker run --rm -v $(pwd):/data ghcr.io/jamon8888/hacienda:latest \
  hacienda pii scan "john@example.com"
```

## Architecture

```text
┌─────────────────────────────────────────────────────────────┐
│                        hacienda                              │
│  ┌────────────────────────────────────────────────────────┐  │
│  │ pub mod extract  { ... }       // 97+ format extraction │  │
│  │ pub mod enrich   { ... }       // NER/classify/caption  │  │
│  │ pub use hacienda_core::*;      // PII/Compliance/Audit  │  │
│  │ pub mod cli { ... }            // hacienda CLI          │  │
│  │ pub mod api { ... }            // REST API + PII routes │  │
│  └────────────────────────────────────────────────────────┘  │
├─────────────────────────────────────────────────────────────┤
│                    hacienda-core (private)                   │
│  ┌──────────┐ ┌──────────┐ ┌──────────┐ ┌──────────────┐    │
│  │   PII    │ │Redaction │ │Compliance│ │   Audit      │    │
│  │ Pipeline │ │ Engine   │ │Generator │ │  Chain       │    │
│  └──────────┘ └──────────┘ └──────────┘ └──────────────┘    │
│  ┌──────────┐ ┌──────────┐ ┌──────────┐ ┌──────────────┐    │
│  │ Review   │ │ Glossary │ │ Profiles │ │  Extraction  │    │
│  │ Queue    │ │ Linker   │ │ PCI/HIPAA│ │  Engine      │    │
│  └──────────┘ └──────────┘ └──────────┘ └──────────────┘    │
└─────────────────────────────────────────────────────────────┘
```

**Built to be extended** — the pipeline is driven entirely through public extension points, so new detectors, formats, and post-processing steps plug in without touching the core:

- `PostProcessor` trait (Late stage, priority 60)
- `NerBackend` trait for custom GLiNER models
- `RedactionConfig::custom_terms` / `custom_patterns`
- Global plugin registries for OCR, extraction, embedding, reranking, tokenizing, validation, and rendering backends

## Configuration

```toml
# hacienda.toml
[extraction]
output_format = "json"

[pii]
enabled = true
redaction_profile = "GDPR"  # PCI, HIPAA, GDPR, Default, Custom
model = { enabled = true, model_id = "fastino/GLiNER2-Guardrails-PII-Multi" }

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
```

## API Reference

### Rust

```rust
use hacienda::{HaciendaFacade, HaciendaFacadeConfig, ExtractInput};

let facade = HaciendaFacade::new(HaciendaFacadeConfig {
    extraction: ExtractionConfig::default(),
    pii: Some(PipelineConfig::default()),
    compliance: Some(ComplianceConfig::default()),
    audit: Some(AuditConfig::default()),
    review: Some(ReviewConfig::default()),
    glossary: Some(GlossaryConfig::default()),
})?;

let result = facade.process(ExtractInput::from_uri("contract.pdf")).await?;
```

### Python

```python
import hacienda

facade = hacienda.HaciendaFacade()
result = facade.process(hacienda.ExtractInput.from_uri("contract.pdf"))
print(result.pii.redacted_text)
```

### REST API

```bash
curl -X POST http://localhost:8080/v1/pii/scan \
  -H "Content-Type: application/json" \
  -d '{"text": "Contact john@example.com"}'

# Response
{
  "entities": [{"category": "Email", "start": 8, "end": 24, "confidence": 0.99}],
  "redacted": "Contact [EMAIL:john@example.com]"
}
```

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

### Release

```bash
git tag -s v0.1.0 -m "hacienda 0.1.0"
git push origin v0.1.0
gh workflow run publish.yaml -f tag=v0.1.0
```

## Bindings

### REST API clients (real, generated from the live OpenAPI schema)

| Language   | Package                       | Status |
| ---------- | ------------------------------ | ------ |
| Python     | `sdks/python` (not yet on PyPI) | ✅     |
| TypeScript | `sdks/typescript` (not yet on npm) | ✅     |

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
