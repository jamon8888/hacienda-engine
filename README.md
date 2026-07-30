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

| Language       | Package                                | Status |
| -------------- | -------------------------------------- | ------ |
| Python         | `pip install hacienda`                 | ✅     |
| Node.js        | `npm i @hacienda/hacienda`             | ✅     |
| WASM           | `@hacienda/hacienda-wasm`              | ✅     |
| Ruby           | `gem install hacienda`                 | ✅     |
| PHP            | `composer require hacienda/hacienda`   | ✅     |
| Go             | `go get github.com/jamon8888/hacienda` | ✅     |
| Java           | Maven Central                          | ✅     |
| C#             | NuGet                                  | ✅     |
| Elixir         | Hex.pm                                 | ✅     |
| Dart           | pub.dev                                | ✅     |
| Kotlin/Android | Maven Central                          | ✅     |
| Swift          | SPM                                    | ✅     |
| Zig            | GitHub Releases                        | ✅     |
| C FFI          | GitHub Releases                        | ✅     |

## License

Apache-2.0 — see [LICENSE](LICENSE) for details.

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md) for guidelines.
