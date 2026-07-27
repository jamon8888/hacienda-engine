# Design Spec: hacienda — xberg + PII Distribution

**Date:** 2026-07-27
**Status:** Approved
**Supersedes:** Plan 04 (Facade Integration)

---

## Executive Summary

**hacienda** is a downstream distribution crate that re-exports **all of xberg** (core library, CLI, REST API, 14+ language bindings) and adds a **GDPR/DORA/AI Act compliant PII ecosystem** (NER, redaction, audit, compliance, review queue, glossary). It requires **zero changes to xberg core** and leverages xberg's plugin architecture for seamless integration.

**Target:** Single developer maintaining a full polyglot release (Rust, Python, Node, WASM, Ruby, PHP, Go, Java, C#, Elixir, Dart, Kotlin/Android, Swift, Zig, FFI) with one command.

---

## Architecture

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                            hacienda crate (PUBLIC API)                      │
│  ┌──────────────────────────────────────────────────────────────────────┐  │
│  │ pub use xberg::*;              // 100% xberg API surface             │  │
│  │ pub use hacienda_core::*;      // PII/NER/redaction/compliance       │  │
│  │ pub mod cli { ... }            // hacienda CLI (xberg + PII cmds)    │  │
│  │ pub mod api { ... }            // hacienda API (xberg + /v1/pii/*)   │  │
│  │ pub mod prelude { ... }        // Common imports                     │  │
│  └──────────────────────────────────────────────────────────────────────┘  │
├─────────────────────────────────────────────────────────────────────────────┤
│                           hacienda-core (private)                           │
│  ┌──────────────────┐ ┌──────────────────┐ ┌────────────────────────────┐  │
│  │ hacienda-ner     │ │ hacienda-redact  │ │ hacienda-compliance        │  │
│  │ • GLiNER fine-   │ │ • FPE + 5 modes  │ │ • DPIA / Model Card        │  │
│  │   tunes          │ │ • Custom profiles│ │ • DORA / AI Act checklist  │  │
│  │ • NerBackend     │ │ • Audit log      │ │ • Risk assessment          │  │
│  └──────────────────┘ └──────────────────┘ └────────────────────────────┘  │
│  ┌──────────────────┐ ┌──────────────────┐ ┌────────────────────────────┐  │
│  │ hacienda-audit   │ │ hacienda-review  │ │ hacienda-glossary          │  │
│  │ • Hash chain     │ │ • Queue +        │ │ • Entity linking           │  │
│  │ • FileSink       │ │   Approve/Reject │ │ • Markdown link injection  │  │
│  │ • Export CSV/JSON│ │ • Deadlines      │ │ • Glossary generation      │  │
│  └──────────────────┘ └──────────────────┘ └────────────────────────────┘  │
│  ┌──────────────────────────────────────────────────────────────────────┐  │
│  │ facade.rs — HaciendaFacade: unified extract+PII+compliance pipeline  │  │
│  └──────────────────────────────────────────────────────────────────────┘  │
└─────────────────────────────────────────────────────────────────────────────┘
                                    │
                                    ▼
                    ┌───────────────────────────────┐
                    │         xberg (upstream)      │
                    │ • 97 format extraction        │
                    │ • Plugin system (PostProcessor│
                    │   registries, NerBackend)     │
                    │ • 17+ language bindings       │
                    │ • CLI, REST API, MCP          │
                    └───────────────────────────────┘
```

---

## Extension Points Used (Zero xberg Changes)

| hacienda Feature            | xberg Mechanism                                                     |
| --------------------------- | ------------------------------------------------------------------- |
| Custom NER models           | `NerBackend` trait + custom `PostProcessor` (Late stage)            |
| 42 PII regex patterns       | `RedactionConfig::custom_patterns` + pre-redaction `PostProcessor`  |
| Domain PII (PCI/HIPAA)      | `RedactionConfig::custom_terms` + `EntityCategory::Custom`          |
| Extended redaction modes    | `Late` stage `PostProcessor` (priority > 50)                        |
| Audit log                   | `FileSink` implementing `AuditSink` trait                           |
| Compliance reports          | Standalone `ComplianceGenerator` (no xberg hook needed)             |
| Review queue                | Standalone `ReviewQueue` (in-memory + persistence)                  |
| Entity glossary/links       | `Late` stage `PostProcessor` reading `ExtractedDocument::entities`  |
| Glossary markdown injection | `Late` stage `PostProcessor` rewriting `ExtractedDocument::content` |

---

## Public API Design

### hacienda/src/lib.rs — The Facade

```rust
//! hacienda — xberg + PII/Compliance Distribution
//!
//! `use hacienda::*;` gives you:
//! - All of xberg: extract, enrich, chunk, embed, rerank, caption, CLI, API, bindings
//! - PII pipeline: detect, redact, explain, batch
//! - Compliance: DPIA, Model Card, DORA, AI Act checklists
//! - Audit: hash-chained log, CSV/JSON export
//! - Review: human-in-the-loop queue
//! - Glossary: entity linking, markdown links

// =========================================================================
// 1. RE-EXPORT ALL OF XBERG (features mirror xberg's feature flags)
// =========================================================================
pub use xberg::{
    // Core extraction
    extract, extract_batch, ExtractInput, ExtractionConfig, ExtractionResult,
    ExtractedDocument, Entity, EntityCategory, Chunk, Table, Metadata,

    // Enrichment
    enrich, EnrichedResult, EnrichmentConfig,
    NerEnrichmentConfig, ClassificationEnrichmentConfig, CaptioningEnrichmentConfig,

    // Chunking / Embeddings / Reranking / Captioning
    chunk_text, ChunkingResult,
    embed_texts, embed_texts_async, EmbeddingModelType,
    rerank, rerank_async, RerankedDocument,
    caption_image, caption_image_file, caption_images,

    // Configuration types
    OcrConfig, ChunkingConfig, EmbeddingConfig, RerankerConfig,
    NerConfig, NerBackendKind, RedactionConfig, RedactionStrategy,
    PiiCategory, CaptioningConfig, SummarizationConfig,
    ClassificationConfig, TranslationConfig,

    // Plugins
    register_post_processor, register_ocr_backend,
    register_document_extractor, register_embedding_backend,
    register_reranker_backend, register_tokenizer_backend,
    register_validator, register_renderer,
    PostProcessor, OcrBackend, DocumentExtractor,
    EmbeddingBackend, RerankerBackend, TokenizerBackend,
    Validator, Renderer, ProcessingStage,

    // Types & Errors
    OutputFormat, ExtractionErrorItem, ExtractionSummary,
    Result, XbergError,

    // Feature-gated re-exports
    #[cfg(feature = "ner")] detect_entities,
    #[cfg(feature = "ner")] NerBackend,
    #[cfg(feature = "redaction")] TokenCounter,
    #[cfg(feature = "redaction")] RedactionStrategy,
    #[cfg(feature = "presets")] { Preset, Registry, resolve },
};

// =========================================================================
// 2. HACIENDA PII EXTENSIONS
// =========================================================================
pub use hacienda_core::{
    // PII Pipeline
    PiiPipeline, PipelineConfig, PipelineResult, PipelineEntity,
    PipelineAuditEntry, PipelineMetrics,

    // Redaction
    RedactionMode, RedactionConfig as HaciendaRedactionConfig,
    RedactionProfile, PciProfile, HipaaProfile, CustomProfile,

    // Compliance
    ComplianceGenerator, ComplianceReport, ModelCard, DoraReport,
    ComplianceChecklist, ChecklistItem, AiActRiskLevel,

    // Audit
    AuditChain, FileSink, AuditEntry, AuditSink, EntitySource,
    AuditExporter, ExportFormat as AuditExportFormat,

    // Review Queue
    ReviewQueue, ReviewQueueItem, ReviewRequest, ReviewDecision,
    ReviewStatus, Priority, QueueStats, ReviewConfig,

    // Glossary / Entity Linking
    EntityGlossary, GlossaryEntry, generate_markdown_links,
    GlossaryConfig, LinkStyle,

    // Config
    HaciendaConfig, HaciendaFacadeConfig,
};

// =========================================================================
// 3. UNIFIED FACADE (single entry point for extract + PII + compliance)
// =========================================================================
pub use hacienda_core::facade::HaciendaFacade;
```

### Feature Flags (Mirror xberg + PII)

```toml
# hacienda/Cargo.toml
[features]
default = ["xberg-full", "pii", "compliance", "audit", "review", "glossary"]

# xberg feature passthrough
xberg-full = [
    "xberg/full",          # all xberg features
    "xberg/tokio-runtime", # needed for async API
]

# hacienda features
pii = ["hacienda-core/pii"]
compliance = ["hacienda-core/compliance", "pii"]
audit = ["hacienda-core/audit"]
review = ["hacienda-core/review"]
glossary = ["hacienda-core/glossary"]

# Dev
dev = ["pii", "compliance", "audit", "review", "glossary"]
```

---

## hacienda-core Architecture

### Crate Structure

```
hacienda-core/
├── Cargo.toml
├── src/
│   ├── lib.rs                 # Re-exports
│   ├── config.rs              # HaciendaConfig, HaciendaFacadeConfig
│   ├── facade.rs              # HaciendaFacade (unified pipeline)
│   ├── error.rs               # HaciendaError
│   │
│   ├── pii/
│   │   ├── mod.rs
│   │   ├── pipeline.rs        # PiiPipeline (wraps pii-pipeline crate)
│   │   ├── config.rs          # PipelineConfig, RedactionProfile
│   │   ├── profiles.rs        # PCI, HIPAA, GDPR, Custom profiles
│   │   └── xberg_integration.rs # PostProcessor registration
│   │
│   ├── redaction/
│   │   ├── mod.rs
│   │   ├── engine.rs          # Extended redaction engine
│   │   ├── fpe.rs             # FPE encryption (AES-GCM-SIV)
│   │   └── patterns.rs        # 42 built-in + custom patterns
│   │
│   ├── compliance/
│   │   ├── mod.rs
│   │   ├── generator.rs       # ComplianceGenerator
│   │   ├── dpia.rs            # DPIA document
│   │   ├── model_card.rs      # Model Card (AI Act)
│   │   ├── dora.rs            # DORA incident report
│   │   ├── ai_act.rs          # AI Act risk assessment
│   │   └── checklist.rs       # GDPR/AI Act/DORA checklists
│   │
│   ├── audit/
│   │   ├── mod.rs
│   │   ├── chain.rs           # AuditChain (blake3 hash chain)
│   │   ├── sink.rs            # FileSink, AuditSink trait
│   │   ├── exporter.rs        # CSV, JSON, JSONL export
│   │   └── verifier.rs        # Chain integrity verification
│   │
│   ├── review/
│   │   ├── mod.rs
│   │   ├── queue.rs           # ReviewQueue (Mutex<Vec<>>)
│   │   ├── item.rs            # ReviewQueueItem, ReviewRequest
│   │   ├── decision.rs        # ReviewDecision, ReviewStatus
│   │   └── config.rs          # ReviewConfig (deadlines, auto-assign)
│   │
│   ├── glossary/
│   │   ├── mod.rs
│   │   ├── entity_linker.rs   # EntityGlossary + fuzzy matching
│   │   ├── markdown.rs        # generate_markdown_links
│   │   └── config.rs          # GlossaryConfig, LinkStyle
│   │
│   └── facade.rs              # HaciendaFacade (unified pipeline)
```

### hacienda-core/src/facade.rs — Unified Pipeline

```rust
use xberg::{extract, ExtractInput, ExtractionConfig, ExtractionResult};
use hacienda_core::pii::{PiiPipeline, PipelineConfig, PipelineResult};
use hacienda_core::compliance::ComplianceGenerator;
use hacienda_core::audit::{AuditChain, FileSink};
use hacienda_core::review::ReviewQueue;
use hacienda_core::glossary::{EntityGlossary, generate_markdown_links};

pub struct HaciendaFacade {
    extraction_config: ExtractionConfig,
    pii_pipeline: Option<PiiPipeline>,
    compliance: Option<ComplianceGenerator>,
    audit_chain: Option<Arc<Mutex<AuditChain>>>,
    review_queue: Option<Arc<ReviewQueue>>,
    glossary: Option<Arc<Mutex<EntityGlossary>>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HaciendaFacadeConfig {
    pub extraction: ExtractionConfig,
    pub pii: Option<PipelineConfig>,
    pub compliance: Option<ComplianceConfig>,
    pub audit: Option<AuditConfig>,
    pub review: Option<ReviewConfig>,
    pub glossary: Option<GlossaryConfig>,
}

impl HaciendaFacade {
    pub fn new(config: HaciendaFacadeConfig) -> Result<Self, HaciendaError> {
        let pii_pipeline = config.pii.as_ref()
            .map(|c| PiiPipeline::new(c.clone()))
            .transpose()?;

        let compliance = config.compliance.as_ref()
            .map(|c| ComplianceGenerator::new(c.model_name.clone()));

        let audit_chain = config.audit.as_ref()
            .map(|c| Arc::new(Mutex::new(AuditChain::new(c.config_hash.clone()))));

        let review_queue = config.review.as_ref()
            .map(|c| Arc::new(ReviewQueue::new(c.clone())));

        let glossary = config.glossary.as_ref()
            .map(|c| Arc::new(Mutex::new(EntityGlossary::new(c.clone()))));

        Ok(Self {
            extraction_config: config.extraction,
            pii_pipeline,
            compliance,
            audit_chain,
            review_queue,
            glossary,
        })
    }

    /// Single-shot: extract → PII → compliance → audit → review → glossary
    pub async fn process(&self, input: ExtractInput) -> Result<HaciendaResult, HaciendaError> {
        // 1. Extract (97 formats)
        let extraction = extract(input, &self.extraction_config).await?;

        // 2. PII Pipeline (optional)
        let pii_result = if let Some(pipeline) = &self.pii_pipeline {
            let text = extraction.results.first().map(|r| r.content.as_str()).unwrap_or("");
            let result = pipeline.process(text)?;

            // Log to audit chain
            if let Some(chain) = &self.audit_chain {
                let mut guard = chain.lock().unwrap();
                for entity in &result.entities {
                    let entry = AuditEntry::from_pii_entity(entity, &result);
                    guard.append(entry)?;
                }
            }

            // Submit to review queue if high-risk
            if let Some(queue) = &self.review_queue {
                for entity in &result.entities {
                    if entity.confidence < 0.5 || entity.category == "Custom" {
                        let req = ReviewRequest::from_pii_entity(entity);
                        queue.submit(req);
                    }
                }
            }

            Some(result)
        } else { None };

        // 3. Compliance (optional)
        let compliance_report = if let Some(comp) = &self.compliance {
            Some(comp.full_report().await?)
        } else { None };

        // 4. Glossary linking (optional)
        let glossary_links = if let Some(glossary) = &self.glossary {
            let mut guard = glossary.lock().unwrap();
            for entity in pii_result.as_ref().into_iter().flat_map(|r| &r.entities) {
                guard.insert(entity);
            }
            Some(guard.generate_links(&extraction.results[0].content)?)
        } else { None };

        Ok(HaciendaResult {
            extraction,
            pii: pii_result,
            compliance: compliance_report,
            audit_log: self.audit_chain.as_ref().map(|c| c.lock().unwrap().entries().to_vec()),
            review_count: self.review_queue.as_ref().map(|q| q.stats().pending).unwrap_or(0),
            glossary_links,
        })
    }

    /// Batch processing
    pub async fn process_batch(&self, inputs: Vec<ExtractInput>) -> Result<Vec<HaciendaResult>, HaciendaError> {
        let mut results = Vec::with_capacity(inputs.len());
        for input in inputs {
            results.push(self.process(input).await?);
        }
        Ok(results)
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct HaciendaResult {
    pub extraction: ExtractionResult,
    pub pii: Option<PipelineResult>,
    pub compliance: Option<ComplianceReport>,
    pub audit_log: Option<Vec<AuditEntry>>,
    pub review_count: usize,
    pub glossary_links: Option<String>,
}
```

---

## xberg Integration (Plugin Registration)

### hacienda-core/src/pii/xberg_integration.rs

```rust
use xberg::{
    plugins::{PostProcessor, ProcessingStage, register_post_processor},
    ExtractionConfig, ExtractedDocument, Result, XbergError,
};
use hacienda_core::pii::{PiiPipeline, PipelineConfig};
use std::sync::Arc;

/// PostProcessor that runs PII pipeline as Late stage
pub struct HaciendaPiiProcessor {
    pipeline: Arc<PiiPipeline>,
}

impl HaciendaPiiProcessor {
    pub fn new(config: PipelineConfig) -> Result<Self, String> {
        Ok(Self {
            pipeline: Arc::new(PiiPipeline::new(config)?),
        })
    }
}

impl Plugin for HaciendaPiiProcessor {
    fn name(&self) -> &str { "hacienda-pii" }
    fn version(&self) -> String { env!("CARGO_PKG_VERSION").into() }
    fn description(&self) -> &str { "Hacienda PII detection & redaction" }
    fn author(&self) -> &str { "hacienda team" }
}

#[async_trait]
impl PostProcessor for HaciendaPiiProcessor {
    fn processing_stage(&self) -> ProcessingStage { ProcessingStage::Late }
    fn priority(&self) -> i32 { 60 } // After built-in redaction (50)

    fn should_process(&self, doc: &ExtractedDocument, config: &ExtractionConfig) -> bool {
        doc.content.contains(|c: char| c.is_ascii_alphanumeric()) &&
        config.redaction.is_some()
    }

    async fn process(&self, doc: &mut ExtractedDocument, config: &ExtractionConfig) -> Result<()> {
        let text = &doc.content;
        if text.is_empty() { return Ok(()); }

        let result = self.pipeline.process(text)
            .map_err(|e| XbergError::Other(e))?;

        // Apply redaction to document
        doc.content = result.redacted_text.clone();

        // Add entities to document metadata
        let entities: Vec<xberg::types::Entity> = result.entities.iter().map(|e| {
            xberg::types::Entity {
                text: text[e.start as usize..e.end as usize].to_string(),
                category: e.category.clone(),
                confidence: e.confidence,
                start: e.start,
                end: e.end,
                source: "hacienda-pii".into(),
            }
        }).collect();
        doc.entities = Some(entities);

        // Add redaction report
        doc.metadata.insert("hacienda_pii_report".into(),
            serde_json::to_value(&result).unwrap_or_default());

        Ok(())
    }
}

/// Register hacienda PII processor with xberg
pub fn register_hacienda_pii(config: PipelineConfig) -> Result<(), String> {
    let processor = HaciendaPiiProcessor::new(config)?;
    register_post_processor(Arc::new(processor))
        .map_err(|e| format!("Failed to register hacienda PII processor: {}", e))
}
```

### Auto-Registration via HaciendaFacade

```rust
impl HaciendaFacade {
    pub fn new(config: HaciendaFacadeConfig) -> Result<Self, HaciendaError> {
        // ... (previous init)

        // Register PII processor with xberg if PII enabled
        if let Some(pii_config) = &config.pii {
            hacienda_core::pii::xberg_integration::register_hacienda_pii(pii_config.clone())?;
        }

        Ok(Self { ... })
    }
}
```

---

## Binding Generation (alef.toml for hacienda)

### hacienda/alef.toml

```toml
alef_version = "0.44.0"
extra_clippy_allows = ["redundant_field_names"]
languages = [
  "python", "node", "ruby", "php", "ffi", "go", "java", "csharp",
  "elixir", "wasm", "dart", "jni", "kotlin_android", "swift", "zig"
]

[workspace.dto]
python = "dataclass"
node = "interface"
ruby = "struct"
php = "readonly-class"
elixir = "struct"
go = "struct"
java = "record"
csharp = "record"

[workspace.generate]
bindings = true
errors = true
configs = true
async_wrappers = true
type_conversions = true
package_metadata = true
public_api = true

[workspace.generate.overrides]
wasm = { async_wrappers = false }
java = { async_wrappers = false }
kotlin_android = { async_wrappers = false }

[workspace.docs]
reference_output = "docs-site/src/content/docs/reference"
cli_sources = [
  "hacienda/src/cli.rs",
  "hacienda-core/src/cli_overrides.rs",
]
mcp_sources = [
  "hacienda-core/src/mcp/server.rs",
]
skills = { template_dir = "templates/docs/skills", output_dir = ".codex/skills" }

[workspace.sync]
extra_paths = [
  "hacienda-python/src/__init__.py",
  "hacienda-python/pyproject.toml",
  "hacienda-node/package.json",
  "packages/go/go.mod",
  "hacienda-node/npm/package.json",
  "packages/ruby/hacienda.gemspec",
  "packages/java/pom.xml",
  "packages/csharp/Hacienda/Hacienda.csproj",
  "packages/elixir/mix.exs",
  "packages/dart/pubspec.yaml",
  "packages/zig/build.zig.zon",
  "packages/swift/Package.swift",
]

[[crates]]
name = "hacienda"
core_import = "hacienda"
error_type = "HaciendaError"

sources = [
  "src/lib.rs",
  "src/cli.rs",
  "src/api.rs",
  "src/prelude.rs",
  "src/config.rs",
  "hacienda-core/src/lib.rs",
  "hacienda-core/src/config.rs",
  "hacienda-core/src/facade.rs",
  "hacienda-core/src/error.rs",
  "hacienda-core/src/pii/mod.rs",
  "hacienda-core/src/pii/pipeline.rs",
  "hacienda-core/src/pii/config.rs",
  "hacienda-core/src/pii/profiles.rs",
  "hacienda-core/src/pii/xberg_integration.rs",
  "hacienda-core/src/redaction/mod.rs",
  "hacienda-core/src/redaction/engine.rs",
  "hacienda-core/src/redaction/fpe.rs",
  "hacienda-core/src/redaction/patterns.rs",
  "hacienda-core/src/compliance/mod.rs",
  "hacienda-core/src/compliance/generator.rs",
  "hacienda-core/src/compliance/dpia.rs",
  "hacienda-core/src/compliance/model_card.rs",
  "hacienda-core/src/compliance/dora.rs",
  "hacienda-core/src/compliance/ai_act.rs",
  "hacienda-core/src/compliance/checklist.rs",
  "hacienda-core/src/audit/mod.rs",
  "hacienda-core/src/audit/chain.rs",
  "hacienda-core/src/audit/sink.rs",
  "hacienda-core/src/audit/exporter.rs",
  "hacienda-core/src/audit/verifier.rs",
  "hacienda-core/src/review/mod.rs",
  "hacienda-core/src/review/queue.rs",
  "hacienda-core/src/review/item.rs",
  "hacienda-core/src/review/decision.rs",
  "hacienda-core/src/review/config.rs",
  "hacienda-core/src/glossary/mod.rs",
  "hacienda-core/src/glossary/entity_linker.rs",
  "hacienda-core/src/glossary/markdown.rs",
  "hacienda-core/src/glossary/config.rs",
]

[crates.output]
python = "hacienda-python/src/"
node = "hacienda-node/src/"
ruby = "packages/ruby/ext/hacienda_rb/src/"
php = "hacienda-php/src/"
ffi = "hacienda-ffi/src/"
go = "packages/go/"
elixir = "packages/elixir/native/hacienda_nif/src/"
wasm = "hacienda-wasm/src/"
java = "packages/java/"
csharp = "packages/csharp/src/"
jni = "hacienda-jni/src/"
kotlin_android = "packages/kotlin-android/"
swift = "packages/swift/Sources/Hacienda/"
dart = "packages/dart/lib/src/"
zig = "packages/zig/src/"

[crates.exclude]
types = [
  "HaciendaFacadeConfig",
  "PipelineConfigInternal",
  "AuditChainInternal",
]
fields = [
  "HaciendaFacade::extraction_config",
  "HaciendaFacade::pii_pipeline",
]
methods = [
  "HaciendaFacade::register_internal_processors",
]
```

---

## CI/CD Pipelines

### hacienda/.github/workflows/ci-rust.yaml (from xberg)

```yaml
name: CI - Rust

on:
  push:
    branches: [main]
    paths:
      - "hacienda/**"
      - "hacienda-core/**"
  pull_request:
    branches: [main]

jobs:
  clippy:
    runs-on: ubuntu-24.04-arm
    steps:
      - uses: actions/checkout@v4
      - uses: xberg-io/actions/setup-rust@v1
      - uses: Swatinem/rust-cache@v2
      - run: task check:rust

  test:
    runs-on: ubuntu-24.04-arm
    steps:
      - uses: actions/checkout@v4
      - uses: xberg-io/actions/setup-rust@v1
      - uses: Swatinem/rust-cache@v2
      - run: task test:rust

  feature-matrix:
    runs-on: ubuntu-24.04-arm
    strategy:
      matrix:
        features:
          [
            "xberg-full",
            "pii",
            "compliance",
            "audit",
            "review",
            "glossary",
            "dev",
          ]
    steps:
      - uses: actions/checkout@v4
      - uses: xberg-io/actions/setup-rust@v1
      - uses: Swatinem/rust-cache@v2
      - run: cargo check -p hacienda --features ${{ matrix.features }}

  test-bindings:
    runs-on: ubuntu-24.04-arm
    steps:
      - uses: actions/checkout@v4
      - uses: xberg-io/actions/install-alef@v1
      - run: task test:bindings
```

### hacienda/.github/workflows/publish.yaml (adapted from xberg)

```yaml
name: Release

on:
  workflow_dispatch:
    inputs:
      tag:
        description: "Release tag (e.g., v0.1.0)"
        required: true
      targets:
        description: "Comma-separated targets (default: all)"
        required: false

jobs:
  prepare:
    runs-on: ubuntu-latest
    outputs:
      version: ${{ steps.meta.outputs.version }}
      tag: ${{ steps.meta.outputs.tag }}
    steps:
      - uses: actions/checkout@v4
      - id: meta
        run: |
          TAG="${{ github.event.inputs.tag }}"
          VERSION="${TAG#v}"
          echo "version=$VERSION" >> $GITHUB_OUTPUT
          echo "tag=$TAG" >> $GITHUB_OUTPUT

  validate-versions:
    needs: prepare
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: xberg-io/actions/install-alef@v1
      - run: alef publish validate --version ${{ needs.prepare.outputs.version }}

  build-bindings:
    needs: [prepare, validate-versions]
    strategy:
      matrix:
        include:
          # Python wheels (maturin)
          - lang: python
            runner: ubuntu-24.04-arm
          # Node NAPI
          - lang: node
            runner: ubuntu-24.04-arm
          # WASM
          - lang: wasm
            runner: ubuntu-24.04-arm
          # Ruby
          - lang: ruby
            runner: ubuntu-24.04-arm
          # PHP
          - lang: php
            runner: ubuntu-24.04-arm
          # Go FFI
          - lang: go
            runner: ubuntu-24.04-arm
          # C FFI
          - lang: ffi
            runner: ubuntu-24.04-arm
          # Java
          - lang: java
            runner: ubuntu-24.04-arm
          # C#
          - lang: csharp
            runner: ubuntu-24.04-arm
          # Elixir
          - lang: elixir
            runner: ubuntu-24.04-arm
          # Dart
          - lang: dart
            runner: ubuntu-24.04-arm
          # Kotlin/Android
          - lang: kotlin_android
            runner: ubuntu-24.04-arm
          # Swift
          - lang: swift
            runner: macos-14
          # Zig
          - lang: zig
            runner: ubuntu-24.04-arm
          # CLI binaries
          - lang: cli
            runner: ubuntu-24.04-arm

    runs-on: ${{ matrix.runner }}
    steps:
      - uses: actions/checkout@v4
      - uses: xberg-io/actions/setup-rust@v1
      - uses: xberg-io/actions/install-alef@v1
      - run: alef build --lang ${{ matrix.lang }} --release
      - uses: actions/upload-artifact@v4
        with:
          name: hacienda-${{ matrix.lang }}
          path: build/${{ matrix.lang }}/

  upload-assets:
    needs: [prepare, build-bindings]
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: actions/download-artifact@v4
      - uses: softprops/action-gh-release@v1
        with:
          tag_name: ${{ needs.prepare.outputs.tag }}
          draft: true
          files: build/**/*

  publish:
    needs: [prepare, build-bindings, upload-assets]
    strategy:
      matrix:
        registry:
          [
            crates_io,
            pypi,
            npm,
            rubygems,
            maven,
            nuget,
            hex,
            pub_dev,
            packagist,
            homebrew,
          ]
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: xberg-io/actions/install-alef@v1
      - run: alef publish ${{ matrix.registry }} --version ${{ needs.prepare.outputs.version }}
```

---

## Single-Developer Workflow

### hacienda/Taskfile.yml (from xberg)

```yaml
version: "3"

vars:
  PLAN: docs/superpowers/plans/2026-07-27-hacienda.md
  SPEC: docs/superpowers/specs/2026-07-27-hacienda-design.md

tasks:
  # Setup
  setup:
    desc: Install all toolchains
    cmds:
      - rustup toolchain install nightly
      - cargo install cargo-alef cargo-poly task cargo-hack
      - poly hooks install

  # Development
  dev:
    desc: Start development loop
    cmds:
      - task check
      - task test

  # Building
  build:
    desc: Build Rust core (debug)
    cmds:
      - cargo build -p hacienda -p hacienda-core

  build:release:
    desc: Build Rust core (release)
    cmds:
      - cargo build -p hacienda -p hacienda-core --release

  build:bindings:
    desc: Build all language bindings
    cmds:
      - alef build --all-targets --release

  build:all:
    desc: Build core + all bindings
    cmds:
      - task build:release
      - task build:bindings

  # Testing
  test:
    desc: Run Rust tests
    cmds:
      - cargo test -p hacienda -p hacienda-core

  test:bindings:
    desc: Run binding tests
    cmds:
      - alef test --all-languages

  test:all:
    desc: Full test suite
    cmds:
      - task test
      - task test:bindings

  test:e2e:
    desc: Run E2E tests
    cmds:
      - alef e2e test --all-languages

  test:cov:
    desc: Test with coverage
    cmds:
      - cargo llvm-cov -p hacienda -p hacienda-core --lcov --output-path lcov.info
      - alef test --coverage --all-languages

  # Linting
  lint:
    desc: Run poly lint (all languages)
    cmds:
      - poly lint .

  lint:check:
    desc: Check lint (no fix)
    cmds:
      - poly lint --check .

  fmt:
    desc: Format all code
    cmds:
      - poly fmt --fix .

  fmt:check:
    desc: Check format (no fix)
    cmds:
      - poly fmt --check .

  check:
    desc: Full check (fmt + lint)
    cmds:
      - task fmt:check
      - task lint:check

  # alef
  alef:generate:
    desc: Regenerate all bindings
    cmds:
      - alef all --clean

  alef:verify:
    desc: Verify bindings are up to date
    cmds:
      - alef verify --exit-code

  alef:build:
    desc: Build all bindings
    cmds:
      - alef build --all-targets

  alef:sync:
    desc: Sync versions from Cargo.toml
    cmds:
      - alef sync-versions

  alef:docs:
    desc: Generate docs
    cmds:
      - alef docs

  # E2E per language
  e2e:lang:
    desc: Run E2E for specific language
    vars:
      LANG: python
    cmds:
      - alef e2e test --lang {{.LANG}}

  e2e:all:
    desc: Full E2E matrix
    cmds:
      - task e2e:generate
      - task e2e:build
      - task e2e:test

  e2e:generate:
    cmds:
      - alef e2e generate

  e2e:build:
    cmds:
      - alef build --all-targets --release

  e2e:test:
    cmds:
      - alef test --e2e --all-languages

  # Version management
  versions:sync:
    desc: Sync version from Cargo.toml to all packages
    cmds:
      - alef sync-versions

  update:
    desc: Update all deps (minor/patch)
    cmds:
      - cargo upgrade --manifest-path hacienda/Cargo.toml --manifest-path hacienda-core/Cargo.toml --allow-stale
      - task versions:sync

  upgrade:
    desc: Upgrade all deps (major allowed)
    cmds:
      - cargo upgrade --manifest-path hacienda/Cargo.toml --manifest-path hacienda-core/Cargo.toml
      - task versions:sync

  # Release
  release:
    desc: Create release tag
    vars:
      VERSION: "0.1.0"
    cmds:
      - git tag -a v{{.VERSION}} -m "Release v{{.VERSION}}"
      - git push origin v{{.VERSION}}
      - gh workflow run publish.yaml -f tag=v{{.VERSION}}
```

---

## Reuse of Existing PII Ecosystem (13 crates)

```
xberg-pii-ecosystem (13 crates)
    │
    ├── pii-pipeline     ──► hacienda-core/pii/pipeline.rs (wrapper)
    ├── pii-config       ──► hacienda-core/pii/config.rs (re-export)
    ├── pii-compliance   ──► hacienda-core/compliance/ (direct use)
    ├── pii-audit        ──► hacienda-core/audit/ (direct use)
    ├── pii-review       ──► hacienda-core/review/ (direct use)
    ├── pii-redaction    ──► hacienda-core/redaction/ (direct use)
    ├── pii-fastino      ──► hacienda-core/pii/fastino_backend.rs
    ├── pii-regex        ──► hacienda-core/redaction/patterns.rs (merged)
    ├── pii-merge        ──► hacienda-core/pii/merge.rs (internal)
    ├── pii-api          ──► hacienda/src/api.rs (extended)
    ├── pii-cli          ──► hacienda/src/cli.rs (extended)
    ├── pii-mcp-server   ──► hacienda-core/src/mcp/server.rs (new)
    └── pii-wasm         ──► hacienda-wasm (generated via alef)
```

---

## Maintenance Burden

| Activity                  | Frequency | Time                                                            |
| ------------------------- | --------- | --------------------------------------------------------------- |
| Update xberg version      | Weekly    | 1 min (`task upgrade && task check`)                            |
| Cut release               | Monthly   | 0 min (`git tag vX.Y.Z && git push && gh workflow run publish`) |
| Add new language binding  | Once      | 2 hours (add to `alef.toml`, run `task alef:generate`)          |
| Fix CI breakage           | Rare      | 15 min (read logs, fix, push)                                   |
| Update Docker base images | Quarterly | 10 min (Renovate PRs)                                           |
| Update dependencies       | Quarterly | 5 min (`task upgrade && task check`)                            |

**Total: ~2 hours/month**

---

## Approval Request

This spec covers:

1. ✅ Architecture (zero xberg changes, plugin-based extension)
2. ✅ Public API design (re-export xberg + PII extensions)
3. ✅ hacienda-core internal structure
4. ✅ xberg integration via plugin registration
5. ✅ Binding generation via alef.toml (full polyglot)
6. ✅ CI/CD pipelines (check, test, release)
7. ✅ Single-developer workflow (task, alef, poly)
8. ✅ Reuse of all 13 existing PII ecosystem crates
9. ✅ Directory layout and maintenance plan

**Ready to proceed to implementation plan?**
