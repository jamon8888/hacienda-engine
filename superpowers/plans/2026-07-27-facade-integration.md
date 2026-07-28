# xberg Facade & Core PII Integration — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Create a unified `xberg-facade` crate that chains xberg document processing with pii-ecosystem PII detection/redaction, and add PII feature flags to xberg core.

**Architecture:** Two parallel workspaces — pii-ecosystem gets a new `xberg-facade` crate that depends on xberg (path ref) + pii-pipeline; xberg core gets optional `pii-*` cargo features that gate PII dependencies. The facade chains: extract → chunk → embed → rerank → caption → PII.

**Tech Stack:** Rust 2021 (pii-ecosystem), Rust 2024 (xberg), axum 0.7, serde, thiserror, tokio, pii-pipeline, pii-config, pii-compliance

## Global Constraints

- pii-ecosystem workspace: edition 2021, resolver 2, workspace root `/home/jamin/Documents/xberg-pii-ecosystem/`
- xberg workspace: edition 2024, workspace root `/home/jamin/Documents/xberg/`
- Toolchain: nightly via `/home/jamin/.rustup/toolchains/nightly-x86_64-unknown-linux-gnu/bin/cargo`
- All code must compile with `cargo check` (zero errors, warnings acceptable)
- Feature flags must be additive (no breaking changes when features are off)
- Path dependencies between workspaces: `xberg-facade` → `xberg` via `path = "../../xberg"`

---

## Track A: xberg-facade Crate (pii-ecosystem workspace)

### Task A1: Scaffold xberg-facade crate

**Files:**

- Create: `crates/xberg-facade/Cargo.toml`
- Create: `crates/xberg-facade/src/lib.rs`
- Create: `crates/xberg-facade/src/config.rs`
- Create: `crates/xberg-facade/src/error.rs`
- Modify: `/home/jamin/Documents/xberg-pii-ecosystem/Cargo.toml` (add member)

**Interfaces:**

- Consumes: xberg types (`ExtractInput`, `ExtractionConfig`, `ChunkingConfig`, `EmbeddingConfig`, `RerankerConfig`, `CaptioningConfig`), pii-pipeline (`PiiPipeline`, `PipelineResult`), pii-config (`PipelineConfig`)
- Produces: `XbergFacade`, `XbergFacadeConfig`, `FacadeResult`, `FacadeError`

- [ ] **Step 1: Add workspace member**

Edit `/home/jamin/Documents/xberg-pii-ecosystem/Cargo.toml`, add `"crates/xberg-facade"` to `[workspace.members]`.

- [ ] **Step 2: Create Cargo.toml**

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

- [ ] **Step 3: Create error.rs**

```rust
use thiserror::Error;

#[derive(Debug, Error)]
pub enum FacadeError {
    #[error("extraction failed: {0}")]
    Extraction(String),

    #[cfg(feature = "pii")]
    #[error("pii pipeline failed: {0}")]
    Pii(String),

    #[error("configuration error: {0}")]
    Config(String),
}

impl From<xberg::XbergError> for FacadeError {
    fn from(e: xberg::XbergError) -> Self {
        FacadeError::Extraction(e.to_string())
    }
}
```

- [ ] **Step 4: Create config.rs**

```rust
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct XbergFacadeConfig {
    #[serde(default)]
    pub extraction: xberg::ExtractionConfig,

    #[serde(default)]
    pub chunking: Option<xberg::ChunkingConfig>,

    #[serde(default)]
    pub embedding: Option<xberg::EmbeddingConfig>,

    #[serde(default)]
    pub reranking: Option<xberg::RerankerConfig>,

    #[serde(default)]
    pub captioning: Option<xberg::CaptioningConfig>,

    #[cfg(feature = "pii")]
    #[serde(default)]
    pub pii: Option<pii_config::PipelineConfig>,
}

impl Default for XbergFacadeConfig {
    fn default() -> Self {
        Self {
            extraction: xberg::ExtractionConfig::default(),
            chunking: None,
            embedding: None,
            reranking: None,
            captioning: None,
            #[cfg(feature = "pii")]
            pii: None,
        }
    }
}
```

- [ ] **Step 5: Create lib.rs (skeleton)**

```rust
pub mod config;
pub mod error;

pub use config::XbergFacadeConfig;
pub use error::FacadeError;

use xberg::ExtractInput;

#[derive(Debug, Clone, serde::Serialize)]
pub struct FacadeResult {
    pub extraction: xberg::ExtractionResult,
    pub chunks: Option<Vec<ChunkInfo>>,
    pub embeddings: Option<Vec<Vec<f32>>>,
    pub reranked: Option<Vec<RerankInfo>>,
    pub captions: Option<Vec<String>>,
    #[cfg(feature = "pii")]
    pub pii: Option<PiiInfo>,
    pub metadata: FacadeMetadata,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct ChunkInfo {
    pub content: String,
    pub index: usize,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct RerankInfo {
    pub index: usize,
    pub score: f32,
    pub document: String,
}

#[cfg(feature = "pii")]
#[derive(Debug, Clone, serde::Serialize)]
pub struct PiiInfo {
    pub redacted_text: String,
    pub entities_count: usize,
    pub audit_entries_count: usize,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct FacadeMetadata {
    pub input_mime: Option<String>,
    pub processing_time_ms: u64,
    pub pii_enabled: bool,
}

pub struct XbergFacade {
    config: XbergFacadeConfig,
    #[cfg(feature = "pii")]
    pii_pipeline: Option<pii_pipeline::PiiPipeline>,
}

impl XbergFacade {
    pub fn new(config: XbergFacadeConfig) -> Result<Self, FacadeError> {
        #[cfg(feature = "pii")]
        let pii_pipeline = match &config.pii {
            Some(pii_config) => {
                let pipeline = pii_pipeline::PiiPipeline::new(pii_config.clone())
                    .map_err(FacadeError::Pii)?;
                Some(pipeline)
            }
            None => None,
        };

        Ok(Self {
            config,
            #[cfg(feature = "pii")]
            pii_pipeline,
        })
    }

    pub async fn process(&self, input: ExtractInput) -> Result<FacadeResult, FacadeError> {
        let start = std::time::Instant::now();

        // 1. Extract
        let extraction = xberg::extract(input, &self.config.extraction).await?;

        // 2. Chunk (optional)
        let chunks = if let Some(chunk_config) = &self.config.chunking {
            let text = &extraction.results.first()
                .map(|r| r.content.as_str())
                .unwrap_or("");
            if !text.is_empty() {
                let result = xberg::chunk_text(text, chunk_config, None)
                    .map_err(|e| FacadeError::Extraction(e.to_string()))?;
                Some(result.chunks.into_iter().map(|c| ChunkInfo {
                    content: c.content,
                    index: c.index,
                }).collect())
            } else {
                None
            }
        } else {
            None
        };

        // 3. Embed (optional)
        let embeddings = if let Some(emb_config) = &self.config.embedding {
            let texts: Vec<String> = chunks.as_ref()
                .map(|cs| cs.iter().map(|c| c.content.clone()).collect())
                .or_else(|| {
                    extraction.results.first()
                        .map(|r| vec![r.content.clone()])
                })
                .unwrap_or_default();
            if !texts.is_empty() {
                let embs = xberg::embed_texts(texts, emb_config)
                    .map_err(|e| FacadeError::Extraction(e.to_string()))?;
                Some(embs)
            } else {
                None
            }
        } else {
            None
        };

        // 4. Rerank (optional, requires query — skip if no query available)
        let reranked = None; // Reranking requires a query; facade provides it via separate method

        // 5. Caption images (optional)
        let captions = if let Some(cap_config) = &self.config.captioning {
            let images: Vec<&[u8]> = extraction.results.iter()
                .flat_map(|r| r.images.iter().map(|img| img.bytes.as_slice()))
                .collect();
            if !images.is_empty() {
                let caps = xberg::caption_images(&images, &cap_config.llm, cap_config.prompt.as_deref())
                    .await
                    .map_err(|e| FacadeError::Extraction(e.to_string()))?;
                Some(caps)
            } else {
                None
            }
        } else {
            None
        };

        // 6. PII (optional)
        #[cfg(feature = "pii")]
        let pii = if let Some(pipeline) = &self.pii_pipeline {
            let text = &extraction.results.first()
                .map(|r| r.content.as_str())
                .unwrap_or("");
            if !text.is_empty() {
                let result = pipeline.process(text)
                    .map_err(FacadeError::Pii)?;
                Some(PiiInfo {
                    redacted_text: result.redacted_text,
                    entities_count: result.entities.len(),
                    audit_entries_count: result.audit_log.len(),
                })
            } else {
                None
            }
        } else {
            None
        };

        let elapsed = start.elapsed().as_millis() as u64;
        let input_mime = extraction.results.first()
            .and_then(|r| r.mime_type.clone());

        Ok(FacadeResult {
            extraction,
            chunks,
            embeddings,
            reranked,
            captions,
            #[cfg(feature = "pii")]
            pii,
            metadata: FacadeMetadata {
                input_mime,
                processing_time_ms: elapsed,
                #[cfg(feature = "pii")]
                pii_enabled: self.pii_pipeline.is_some(),
                #[cfg(not(feature = "pii"))]
                pii_enabled: false,
            },
        })
    }

    pub async fn process_batch(&self, inputs: Vec<ExtractInput>) -> Result<Vec<FacadeResult>, FacadeError> {
        let mut results = Vec::with_capacity(inputs.len());
        for input in inputs {
            results.push(self.process(input).await?);
        }
        Ok(results)
    }
}
```

- [ ] **Step 6: Verify compilation**

Run: `cargo check -p xberg-facade --features pii` from `/home/jamin/Documents/xberg-pii-ecosystem/`
Expected: PASS (0 errors)

- [ ] **Step 7: Commit**

```bash
cd /home/jamin/Documents/xberg-pii-ecosystem
git add crates/xberg-facade/ Cargo.toml
git commit -m "feat: add xberg-facade crate with PII pipeline integration"
```

---

## Track B: xberg Core PII Features

### Task B1: Add PII feature flags to xberg

**Files:**

- Modify: `/home/jamin/Documents/xberg/crates/xberg/Cargo.toml`
- Modify: `/home/jamin/Documents/xberg/crates/xberg/src/lib.rs`

**Interfaces:**

- Consumes: pii-pipeline, pii-config, pii-compliance (path deps from pii-ecosystem)
- Produces: `xberg::pii` module (re-exports), `pii` and `pii-compliance` feature flags

- [ ] **Step 1: Add optional dependencies to xberg Cargo.toml**

Edit `/home/jamin/Documents/xberg/crates/xberg/Cargo.toml`, add to `[dependencies]`:

```toml
pii-pipeline = { path = "../../../../xberg-pii-ecosystem/crates/pii-pipeline", optional = true }
pii-config = { path = "../../../../xberg-pii-ecosystem/crates/pii-config", optional = true }
pii-compliance = { path = "../../../../xberg-pii-ecosystem/crates/pii-compliance", optional = true }
```

- [ ] **Step 2: Add feature flags to xberg Cargo.toml**

Add to `[features]` section:

```toml
pii = ["dep:pii-pipeline", "dep:pii-config"]
pii-compliance = ["dep:pii-compliance", "pii"]
```

- [ ] **Step 3: Add pii module to xberg lib.rs**

Add to `/home/jamin/Documents/xberg/crates/xberg/src/lib.rs`:

```rust
/// PII detection and redaction pipeline integration.
#[cfg(feature = "pii")]
pub mod pii {
    pub use pii_pipeline::{
        PiiPipeline, PipelineResult, PipelineEntity,
        PipelineAuditEntry, PipelineMetrics,
    };
    pub use pii_config::PipelineConfig as PiiPipelineConfig;
}
```

- [ ] **Step 4: Verify compilation (no features)**

Run: `cargo check -p xberg` from `/home/jamin/Documents/xberg/`
Expected: PASS — PII code is gated behind feature flags

- [ ] **Step 5: Verify compilation (with pii feature)**

Run: `cargo check -p xberg --features pii` from `/home/jamin/Documents/xberg/`
Expected: PASS

- [ ] **Step 6: Commit**

```bash
cd /home/jamin/Documents/xberg
git add crates/xberg/Cargo.toml crates/xberg/src/lib.rs
git commit -m "feat: add optional pii feature flags for PII pipeline integration"
```

---

## Track A (continued): Facade Compilation Fix

### Task A2: Fix facade compilation against xberg

After Track B adds the pii features to xberg, the facade crate needs xberg to compile with those features. This task ensures the full dependency chain works.

**Files:**

- May modify: `crates/xberg-facade/Cargo.toml` (adjust xberg features if needed)
- May modify: `crates/xberg-facade/src/lib.rs` (fix type mismatches)

**Interfaces:**

- Consumes: xberg (with pii features), pii-pipeline, pii-config
- Produces: compiling xberg-facade crate

- [ ] **Step 1: Full workspace check**

Run: `cargo check --workspace` from `/home/jamin/Documents/xberg-pii-ecosystem/`
Expected: PASS (0 errors)

If errors occur, fix type mismatches between xberg types and pii-pipeline types.

- [ ] **Step 2: Fix any path dependency issues**

If xberg can't resolve pii-pipeline path, adjust paths in xberg's Cargo.toml. The relative path from xberg to pii-ecosystem is `../../../../xberg-pii-ecosystem/crates/pii-*`.

- [ ] **Step 3: Commit fixes**

```bash
cd /home/jamin/Documents/xberg-pii-ecosystem
git add -A
git commit -m "fix: resolve cross-workspace dependency paths for facade"
```

---

## Track C: Tests

### Task C1: Integration test for facade

**Files:**

- Create: `crates/xberg-facade/tests/facade_tests.rs`

**Interfaces:**

- Consumes: `XbergFacade`, `XbergFacadeConfig`, `ExtractInput`
- Produces: passing integration test

- [ ] **Step 1: Create test file**

```rust
use xberg_facade::{XbergFacade, XbergFacadeConfig};

#[tokio::test]
async fn facade_default_config_compiles() {
    let config = XbergFacadeConfig::default();
    let facade = XbergFacade::new(config).expect("failed to create facade");
    // Just verify construction works
    drop(facade);
}

#[tokio::test]
async fn facade_process_plain_text() {
    let config = XbergFacadeConfig::default();
    let facade = XbergFacade::new(config).unwrap();

    let input = xberg::ExtractInput::from_bytes(
        b"Hello world, this is a test document.".to_vec(),
        "text/plain",
        Some("test.txt".to_string()),
    );

    let result = facade.process(input).await.unwrap();
    assert!(!result.extraction.results.is_empty());
    assert_eq!(result.metadata.processing_time_ms > 0, true);
}

#[cfg(feature = "pii")]
#[tokio::test]
async fn facade_process_with_pii() {
    let config = XbergFacadeConfig {
        pii: Some(pii_config::PipelineConfig::default()),
        ..Default::default()
    };
    let facade = XbergFacade::new(config).unwrap();

    let input = xberg::ExtractInput::from_bytes(
        b"Contact john@example.com or call +1-555-123-4567.".to_vec(),
        "text/plain",
        Some("contact.txt".to_string()),
    );

    let result = facade.process(input).await.unwrap();
    assert!(result.pii.is_some());
    let pii = result.pii.unwrap();
    assert!(pii.entities_count > 0);
    assert!(pii.redacted_text.contains("[EMAIL:") || pii.redacted_text.contains("[PHONE:"));
}
```

- [ ] **Step 2: Run tests**

Run: `cargo test -p xberg-facade` from `/home/jamin/Documents/xberg-pii-ecosystem/`
Expected: PASS

- [ ] **Step 3: Commit**

```bash
cd /home/jamin/Documents/xberg-pii-ecosystem
git add crates/xberg-facade/tests/
git commit -m "test: add integration tests for xberg-facade"
```

---

## Execution Order Summary

```text
Track A (Task A1) ──┐
                     ├──▶ Track A (Task A2: fix compilation) ──▶ Track C (Task C1: tests)
Track B (Task B1) ──┘
```

Tracks A1 and B1 execute **in parallel** (separate workspaces, no shared files).
Track A2 runs after both complete (resolves cross-workspace dependencies).
Track C1 runs last (tests the integrated result).

---

**Plan saved to:** `docs/superpowers/plans/2026-07-27-facade-integration.md`
