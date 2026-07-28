# Design Spec: xberg Facade & Core PII Integration

**Date:** 2026-07-27
**Status:** Approved
**Supersedes:** Plan 04 in `docs/superpowers/plans/04-facade-integration.md`

---

## Goal

Create a unified `xberg-facade` crate that chains xberg's document processing pipeline with the pii-ecosystem's PII detection/redaction/compliance pipeline, and add PII feature flags to xberg core so users can opt-in via Cargo features.

## Architecture

```text
Two parallel workspaces, two agents:

Agent A (pii-ecosystem)          Agent B (xberg core)
┌──────────────────────┐         ┌──────────────────────┐
│  xberg-facade crate  │         │  xberg Cargo.toml    │
│  - XbergFacade       │         │  - pii-* features    │
│  - XbergFacadeConfig │         │  - NerBackendKind    │
│  - FacadeResult      │         │  - pii PostProcessor │
│  Depends on:         │         │  - pii re-exports    │
│    xberg (path)      │────────▶│                      │
│    pii-pipeline      │         └──────────────────────┘
│    pii-config        │
│    pii-compliance    │
└──────────────────────┘
```

## Track A: `xberg-facade` Crate

### Location

`/home/jamin/Documents/xberg-pii-ecosystem/crates/xberg-facade/`

### Cargo.toml

```toml
[package]
name = "xberg-facade"
version.workspace = true
edition.workspace = true
description = "Unified facade for xberg + PII pipeline"
license.workspace = true

[features]
default = ["pii"]
pii = ["dep:pii-pipeline", "dep:pii-config"]
compliance = ["dep:pii-compliance", "pii"]

[dependencies]
xberg = { path = "../../xberg", features = [
    "redaction", "ner", "chunking", "embeddings", "reranker",
    "captioning", "tokio-runtime"
] }
pii-pipeline = { path = "../pii-pipeline", optional = true }
pii-config = { path = "../pii-config", optional = true }
pii-compliance = { path = "../pii-compliance", optional = true }
serde = { workspace = true }
serde_json = { workspace = true }
thiserror = "1"
tokio = { workspace = true }
```

### Public API

```rust
// config.rs
pub struct XbergFacadeConfig {
    pub extraction: xberg::ExtractionConfig,
    pub chunking: Option<xberg::ChunkingConfig>,
    pub embedding: Option<xberg::EmbeddingConfig>,
    pub reranking: Option<xberg::RerankerConfig>,
    pub captioning: Option<xberg::CaptioningConfig>,
    #[cfg(feature = "pii")]
    pub pii: Option<pii_config::PipelineConfig>,
}

// lib.rs
pub struct XbergFacade { config: XbergFacadeConfig }

impl XbergFacade {
    pub fn new(config: XbergFacadeConfig) -> Result<Self, FacadeError>;
    pub async fn process(&self, input: xberg::ExtractInput) -> Result<FacadeResult, FacadeError>;
    pub async fn process_batch(&self, inputs: Vec<xberg::ExtractInput>) -> Result<Vec<FacadeResult>, FacadeError>;
}

pub struct FacadeResult {
    pub extraction: xberg::ExtractionResult,
    pub chunks: Option<Vec<xberg::Chunk>>,
    pub embeddings: Option<Vec<Vec<f32>>>,
    pub reranked: Option<Vec<xberg::RerankedDocument>>,
    pub captions: Option<Vec<String>>,
    #[cfg(feature = "pii")]
    pub pii: Option<pii_pipeline::PipelineResult>,
    pub metadata: FacadeMetadata,
}
```

### Processing Pipeline

```text
ExtractInput
    │
    ▼
xberg::extract()          ──▶  ExtractionResult (97 formats)
    │
    ▼ (if chunking enabled)
xberg::chunk_text()       ──▶  Vec<Chunk>
    │
    ▼ (if embedding enabled)
xberg::embed_texts()      ──▶  Vec<Vec<f32>>
    │
    ▼ (if reranking enabled, requires query)
xberg::rerank()           ──▶  Vec<RerankedDocument>
    │
    ▼ (if captioning enabled, images present)
xberg::caption_images()   ──▶  Vec<String>
    │
    ▼ (if pii feature enabled)
pii_pipeline.process()    ──▶  PipelineResult (redacted text + entities + audit)
    │
    ▼
FacadeResult
```

### Error Handling

```rust
#[derive(Debug, thiserror::Error)]
pub enum FacadeError {
    #[error("extraction failed: {0}")]
    Extraction(#[from] xberg::XbergError),
    #[cfg(feature = "pii")]
    #[error("pii pipeline failed: {0}")]
    Pii(String),
    #[error("configuration error: {0}")]
    Config(String),
}
```

## Track B: xberg Core PII Features

### Location

`/home/jamin/Documents/xberg/crates/xberg/`

### Cargo.toml Changes

Add to `[dependencies]`:

```toml
pii-pipeline = { path = "../../../xberg-pii-ecosystem/crates/pii-pipeline", optional = true }
pii-config = { path = "../../../xberg-pii-ecosystem/crates/pii-config", optional = true }
pii-compliance = { path = "../../../xberg-pii-ecosystem/crates/pii-compliance", optional = true }
```

Add to `[features]`:

```toml
pii = ["dep:pii-pipeline", "dep:pii-config"]
pii-compliance = ["dep:pii-compliance", "pii"]
```

### NerBackendKind Extension

**File:** `src/core/config/ner.rs`

```rust
pub enum NerBackendKind {
    #[default] Onnx,
    Llm,
    Candle,  // NEW — for pii-fastino Candle backend
}
```

### PII PostProcessor

**File:** `src/plugins/processor/builtin/pii.rs` (NEW)

```rust
#[cfg(feature = "pii")]
pub struct PiiPostProcessor;

#[cfg(feature = "pii")]
#[async_trait]
impl PostProcessor for PiiPostProcessor {
    async fn process(&self, doc: &mut ExtractedDocument, config: &ExtractionConfig) -> Result<()> {
        let Some(redaction_config) = config.redaction.as_ref() else { return Ok(()) };
        if doc.content.is_empty() { return Ok(()); }

        let pipeline_config = pii_config::PipelineConfig::default();
        let pipeline = pii_pipeline::PiiPipeline::new(pipeline_config)
            .map_err(|e| XbergError::Other(e))?;
        let result = pipeline.process(&doc.content)
            .map_err(|e| XbergError::Other(e))?;

        doc.redacted_content = Some(result.redacted_text);
        // Map pii entities to xberg entities
        Ok(())
    }

    fn processing_stage(&self) -> ProcessingStage { ProcessingStage::Late }
}
```

### PII Module Re-exports

**File:** `src/lib.rs` (add)

```rust
#[cfg(feature = "pii")]
pub mod pii {
    pub use pii_pipeline::{PiiPipeline, PipelineResult, PipelineEntity, PipelineAuditEntry, PipelineMetrics};
    pub use pii_config::PipelineConfig as PiiPipelineConfig;
}
```

## Integration Tests (Task 3)

Location: `xberg-facade/tests/facade_tests.rs`

```rust
#[tokio::test]
async fn facade_extract_and_pii() {
    let config = XbergFacadeConfig {
        extraction: ExtractionConfig::default(),
        #[cfg(feature = "pii")]
        pii: Some(PiiPipelineConfig::default()),
        ..Default::default()
    };
    let facade = XbergFacade::new(config).unwrap();
    let input = ExtractInput::from_bytes(b"Email john@example.com for info".to_vec(), "text/plain", None);
    let result = facade.process(input).await.unwrap();
    assert!(result.extraction.results[0].content.contains("john@example.com"));
    #[cfg(feature = "pii")]
    {
        let pii = result.pii.unwrap();
        assert!(pii.redacted_text.contains("[EMAIL:"));
    }
}
```

## Documentation (Task 4)

- Migration guide at `docs/migration/pii-integration.md`
- Feature flag documentation
- Config examples

## Candle Version Note

xberg uses candle 0.11, pii-ecosystem uses candle 0.8. The facade crate depends on xberg (which brings candle 0.11) and pii-pipeline (which brings candle 0.8 via pii-fastino). Cargo will resolve both versions in the dependency graph. This is acceptable for now — the two candle versions don't interact at the type level. A future cleanup can unify to one version.

## Acceptance Criteria

- [ ] `cargo check -p xberg-facade --features pii` passes
- [ ] `cargo check -p xberg --features pii` passes
- [ ] `cargo test -p xberg-facade` passes
- [ ] Facade processes text through extract → pii pipeline end-to-end
- [ ] Feature flags correctly gate PII dependencies
