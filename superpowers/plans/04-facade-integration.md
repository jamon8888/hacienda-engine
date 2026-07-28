# Implementation Plan: xberg Facade & Core Integration

**Spec Reference:** `docs/superpowers/specs/2025-07-24-xberg-pii-ecosystem-design.md`  
**Plan Version:** 1.0  
**Target:** `xberg-facade` crate + xberg core integration (PR)  
**Priority:** Should Have (v0.2.0)

---

## 📋 Plan Overview

| Task | Description | Est. Hours | Dependencies |
|------|-------------|------------|--------------|
| 1 | `xberg-facade` crate (unified facade) | 6 | Core pipeline + xberg |
| 2 | xberg core integration (PR) | 6 | xberg repo access |
| 3 | Integration tests (facade + xberg) | 4 | Tasks 1-2 |
| 4 | Documentation + migration guide | 2 | Task 2 |

**Total Estimated:** ~18 hours

---

## 🏗️ Task 1: `xberg-facade` Crate

### Files

```text
crates/xberg-facade/
├── Cargo.toml
├── src/
│   ├── lib.rs
│   ├── config.rs
│   ├── extract.rs
│   ├── enrich.rs
│   ├── caption.rs
│   ├── chunk.rs
│   ├── embed.rs
│   ├── rerank.rs
│   ├── structured.rs
│   └── facade.rs
└── tests/
    └── facade_tests.rs
```

### Cargo.toml

```toml
[package]
name = "xberg-facade"
version = "0.1.0"
edition = "2021"
description = "Unified facade for xberg + PII pipeline"
license = "Apache-2.0"

[dependencies]
xberg = { path = "../../xberg", features = [
    "redaction", "ner-onnx", "redaction-patterns",
    "api", "mcp", "chunking", "embeddings", "reranker",
    "captioning", "translation", "classification", "summarization",
    "url-ingestion", "pdf", "office", "ocr", "chunking-tokenizers"
] }
pii-pipeline = { path = "../pii-pipeline" }
pii-config = { path = "../pii-config" }
pii-compliance = { path = "../pii-compliance" }
serde = { workspace = true }
serde_json = { workspace = true }
tokio = { workspace = true, features = ["rt-multi-thread", "macros", "fs"] }
```

### lib.rs - Unified Facade

```rust
use xberg::{
    extract, extract_batch, detect_mime_type,
    ExtractInput, ExtractionConfig, ExtractionResult,
    enrich::{enrich, EnrichmentConfig, EnrichedResult},
    captioning::{caption_image, caption_image_file, caption_images},
    chunking::{chunk, ChunkConfig, ChunkerType},
    embed_texts, embed_texts_async, EmbeddingConfig,
    rerank, RerankerConfig,
};
use pii_pipeline::{PiiPipeline, PipelineConfig, PipelineResult};
use pii_config::PipelineConfig as PiiPipelineConfig;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct XbergFacadeConfig {
    pub extraction: ExtractionConfig,
    pub enrichment: Option<EnrichmentConfig>,
    pub captioning: Option<CaptioningConfig>,
    pub chunking: Option<ChunkConfig>,
    pub embedding: Option<EmbeddingConfig>,
    pub reranking: Option<RerankerConfig>,
    pub pii: Option<PiiPipelineConfig>,
}

impl Default for XbergFacadeConfig {
    fn default() -> Self {
        Self {
            extraction: ExtractionConfig::default(),
            enrichment: None,
            captioning: None,
            chunking: None,
            embedding: None,
            reranking: None,
            pii: None,
        }
    }
}

#[derive(Debug, Serialize)]
pub struct FacadeResult {
    pub extraction: Option<ExtractionResult>,
    pub enrichment: Option<EnrichedResult>,
    pub captioning: Option<Vec<String>>,
    pub chunks: Option<Vec<ChunkResult>>,
    pub embeddings: Option<Vec<Vec<f32>>>,
    pub reranked: Option<Vec<RerankedDocument>>,
    pub pii: Option<PiiPipelineResult>,
    pub metadata: FacadeMetadata,
}

pub struct XbergFacade {
    config: XbergFacadeConfig,
    pii_pipeline: Option<PiiPipeline>,
}

impl XbergFacade {
    pub fn new(config: XbergFacadeConfig) -> Result<Self> {
        let pii_pipeline = config.pii.as_ref()
            .map(|c| PiiPipeline::new(c.clone()))
            .transpose()?;
        Ok(Self { config, pii_pipeline })
    }

    /// Single entry point for complete document processing
    pub async fn process(&self, input: ExtractInput) -> Result<FacadeResult> {
        let mut result = FacadeResult::default();
        
        // 1. Base extraction (97 formats)
        let extraction = extract(input.clone(), &self.config.extraction).await?;
        result.extraction = Some(extraction.clone());
        
        // 2. Enrichment (classification, NER, summary, translation, QR, etc.)
        if let Some(enrich_config) = &self.config.enrichment {
            let enriched = enrich(&extraction, enrich_config).await?;
            result.enrichment = Some(enriched);
        }
        
        // 3. Captioning images
        if let Some(cap_config) = &self.config.captioning {
            if let Some(images) = &extraction.images {
                let captions = caption_images(
                    &images.iter().map(|i| i.bytes.as_slice()).collect::<Vec<_>>(),
                    &cap_config.llm,
                    cap_config.prompt.as_deref(),
                ).await?;
                result.captioning = Some(captions);
            }
        }
        
        // 4. Chunking
        if let Some(chunk_config) = &self.config.chunking {
            let text = &extraction.results[0].content;
            let chunks = chunk(text, chunk_config)?;
            result.chunks = Some(chunks.into_iter().map(Into::into).collect());
        }
        
        // 5. Embeddings
        if let Some(emb_config) = &self.config.embedding {
            let texts = self.extract_texts_for_embedding(&extraction)?;
            let embeddings = embed_texts(texts, emb_config)?;
            result.embeddings = Some(embeddings);
        }
        
        // 6. Reranking
        if let Some(rerank_config) = &self.config.reranking {
            let docs = self.extract_docs_for_rerank(&extraction)?;
            let reranked = rerank(emb_config.query.clone(), docs, rerank_config)?;
            result.reranked = Some(reranked);
        }
        
        // 7. PII Pipeline (GDPR)
        if let Some(pii_pipeline) = &self.pii_pipeline {
            let text = &extraction.results[0].content;
            let pii_result = pii_pipeline.process(text, &self.config.pii.as_ref().unwrap()).await?;
            result.pii = Some(pii_result);
        }
        
        result.metadata = FacadeMetadata {
            processed_at: Utc::now(),
            input_mime: detect_mime_type(input.uri().unwrap_or_default(), true).ok(),
            processing_time_ms: 0,
        };
        
        Ok(result)
    }
    
    /// Batch processing
    pub async fn process_batch(&self, inputs: Vec<ExtractInput>) -> Result<Vec<FacadeResult>> {
        let results = extract_batch(inputs, &self.config.extraction).await?;
        let mut results = Vec::new();
        for extraction in results {
            let input = ExtractInput::from_bytes(&extraction.results[0].content, None);
            results.push(self.process(input).await?);
        }
        Ok(results)
    }
    
    /// Register custom OCR backend
    pub fn register_ocr_backend(&self, backend: impl xberg::plugins::OcrBackend + 'static) -> Result<()> {
        xberg::plugins::register_ocr_backend(Box::new(backend))
    }
    
    /// Register custom post-processor
    pub fn register_post_processor(&self, processor: impl xberg::plugins::PostProcessor + 'static) -> Result<()> {
        xberg::plugins::register_post_processor(Arc::new(processor))
    }
}
```

### config.rs

```rust
use xberg::{CaptioningConfig, ChunkConfig, EmbeddingConfig, RerankerConfig};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct XbergFacadeConfig {
    pub extraction: ExtractionConfig,
    pub enrichment: Option<EnrichmentConfig>,
    pub captioning: Option<CaptioningConfig>,
    pub chunking: Option<ChunkConfig>,
    pub embedding: Option<EmbeddingConfig>,
    pub reranking: Option<RerankerConfig>,
    pub pii: Option<PiiPipelineConfig>,
}
```

---

## 🔗 Task 2: xberg Core Integration (PR)

### 1. xberg/Cargo.toml - New Features

```toml
# Add to [features] section
pii-fastino = ["dep:pii-fastino", "ner"]
pii-pipeline = ["dep:pii-pipeline", "ner", "redaction", "pii-fastino"]
pii-compliance = ["dep:pii-compliance", "pii-pipeline"]
```

### 2. NerBackendKind Extension

**File:** `crates/xberg/src/core/config/ner.rs`

```rust
pub enum NerBackendKind {
    #[default] Onnx,
    Llm,
    Candle,  // NEW - for Fastino Candle backend
}
```

### 3. NerBackend Implementation

**File:** `crates/xberg/src/text/ner/fastino_backend.rs` (NEW)

```rust
use xberg_gliner::candle::Gliner2Candle;

#[cfg_attr(alef, alef(skip))]
pub struct FastinoBackend {
    model: Mutex<Gliner2Candle>,
}

impl FastinoBackend {
    pub fn new(config: &NerConfig) -> Result<Self> {
        let mut model = Gliner2Candle::from_local(config.model.as_deref().unwrap())?;
        if let Some(adapter_dir) = config.lora_adapter_dir.as_ref() {
            let adapter_name = adapter_dir.file_name().and_then(|n| n.to_str()).unwrap_or("adapter");
            model.load_adapter(adapter_name, adapter_dir)?;
        }
        Ok(Self { model: Mutex::new(model) })
    }
}

#[async_trait]
impl NerBackend for FastinoBackend {
    async fn detect(&self, text: &str, categories: &[EntityCategory]) -> Result<Vec<Entity>> {
        let labels: Vec<&str> = categories.iter().map(category_to_label).collect();
        let model = self.model.lock().unwrap();
        let spans = model.extract_ner(text, &labels, 0.5)?;
        Ok(spans_to_entities(spans))
    }
    
    async fn detect_with_custom(
        &self,
        text: &str,
        categories: &[EntityCategory],
        custom_labels: &[String],
    ) -> Result<Vec<Entity>> {
        let mut all = categories.to_vec();
        for label in custom_labels {
            all.push(EntityCategory::Custom(label.clone()));
        }
        self.detect(text, &all).await
    }
}
```

### 4. Register in make_backend

**File:** `crates/xberg/src/text/ner/mod.rs`

```rust
fn make_backend(config: &NerConfig) -> Result<Arc<dyn NerBackend>> {
    match config.backend {
        NerBackendKind::Onnx => { /* existing */ }
        NerBackendKind::Llm => { /* existing */ }
        NerBackendKind::Candle => {
            #[cfg(feature = "pii-fastino")]
            {
                Ok(Arc::new(crate::text::ner::fastino_backend::FastinoBackend::new(config)?))
            }
            #[cfg(not(feature = "pii-fastino"))]
            { Err(missing_feature("pii-fastino")) }
        }
    }
}
```

### 5. PII Post-Processor

**File:** `crates/xberg/src/plugins/processor/builtin/ner.rs` (extend)

```rust
#[cfg(feature = "pii-pipeline")]
pub fn register_pii_processor() -> Result<()> {
    register_post_processor(Arc::new(PiiPostProcessor))
}

#[cfg(feature = "pii-pipeline")]
struct PiiPostProcessor;

#[cfg(feature = "pii-pipeline")]
#[async_trait]
impl PostProcessor for PiiPostProcessor {
    async fn process(&self, doc: &mut ExtractedDocument, config: &ExtractionConfig) -> Result<()> {
        let Some(ner_config) = config.ner.as_ref() else { return Ok(()) };
        if doc.content.is_empty() { return Ok(()); }
        
        let pipeline = PiiPipeline::new(ner_config.clone())?;
        let result = pipeline.process(&doc.content, ner_config).await?;
        
        doc.entities = Some(result.entities);
        doc.pii_audit_log = Some(result.audit_log);
        doc.redacted_content = Some(result.redacted_text);
        Ok(())
    }
}
```

### 5. WASM NER Backend Kind

**File:** `crates/xberg-wasm/src/lib.rs` (add variant)

```rust
pub enum WasmNerBackendKind {
    Onnx = 0,
    Llm = 1,
    Candle = 2,  // NEW
}
```

### 6. CLI Subcommand

**File:** `crates/xberg-cli/src/commands/pii.rs` (NEW)

```rust
pub fn pii_command() -> Command {
    Command::new("pii")
        .subcommand(scan_command())
        .subcommand(redact_command())
        .subcommand(explain_command())
        .subcommand(compliance_report_command())
        .subcommand(model_card_command())
        .subcommand(human_review_command())
}
```

---

## 🧪 Task 3: Integration Tests

```rust
// tests/facade_integration.rs

#[tokio::test]
async fn full_facade_pii() {
    let config = XbergFacadeConfig {
        extraction: ExtractionConfig { output_format: OutputFormat::Json, ..Default::default() },
        pii: Some(PiiPipelineConfig { redaction_mode: RedactionMode::Pseudonymize, ..Default::default() }),
        ..Default::default()
    };
    let facade = XbergFacade::new(config).unwrap();
    
    let input = ExtractInput::from_uri("testdata/contract.pdf");
    let result = facade.process(input).await.unwrap();
    
    assert!(result.extraction.is_some());
    assert!(result.pii.is_some());
    assert!(result.pii.unwrap().redacted_text.contains("[PERSON:"));
    assert!(result.pii.unwrap().audit_log.is_some());
}

#[tokio::test]
async fn facade_batch() {
    let facade = XbergFacade::new(Default::default()).unwrap();
    let inputs = vec![
        ExtractInput::from_uri("testdata/doc1.pdf"),
        ExtractInput::from_uri("testdata/doc2.pdf"),
    ];
    let results = facade.process_batch(inputs).await.unwrap();
    assert_eq!(results.len(), 2);
}

#[tokio::test]
async fn xberg_ner_candle_backend() {
    let config = NerConfig { backend: NerBackendKind::Candle, ..Default::default() };
    let backend = make_backend(&config).unwrap();
    let entities = backend.detect("Jean Dupont habite à Paris", &[EntityCategory::Person]).await.unwrap();
    assert!(!entities.is_empty());
}
```

---

## 📝 Task 4: Documentation + Migration Guide

### Migration Guide

```markdown
# docs/migration/pii-integration.md

# Migrating to xberg with PII Pipeline

## Enable Features
```toml
[dependencies]
xberg = { version = "1.0", features = ["pii-pipeline", "pii-fastino", "pii-compliance"] }
```

## Config

```toml
[extraction]
# ... existing config

[pii]
redaction_mode = "pseudonymize"
thresholds_file = "models/thresholds.toml"
return_audit_log = true
```

## Usage

```rust
let facade = XbergFacade::new(XbergFacadeConfig {
    extraction: ExtractionConfig::default(),
    pii: Some(PiiPipelineConfig::default()),
}).unwrap();

let result = facade.process(ExtractInput::from_uri("contract.pdf")).await.unwrap();
```

## Conformité RGPD/DORA/AI Act

- **Art. 25 RGPD** : Redaction par défaut, audit log
- **Art. 30 RGPD** : Registre = audit log structuré
- **Art. 32 RGPD** : FPE, audit chain, encryption
- **DORA** : Incident report tool, chaos testing
- **AI Act** : Model card, risk assessment, human oversight

```text

---

## 📦 Delivery Checklist

- [ ] `xberg-facade` crate publishes to crates.io
- [ ] xberg PR merged with PII features
- [ ] `xberg-pii` feature flag works end-to-end
- [ ] `cargo test --workspace` passes
- [ ] `cargo test --package xberg-facade` passes
- [ ] Integration tests pass in CI
- [ ] Migration guide published
- [ ] CHANGELOG.md updated
- [ ] ADR-007: PII integration design

---

## 📅 Timeline

| Week | Focus |
|---|---|
| 1 | xberg-facade crate + config |
| 2 | xberg PR (features, backend, post-processor, WASM) |
| 3 | Integration tests + CI |
| 4 | Docs + migration guide + release |

---

**Plan Status:** Ready for execution  
**Next Step:** Execute Task 1 (xberg-facade crate scaffolding)
