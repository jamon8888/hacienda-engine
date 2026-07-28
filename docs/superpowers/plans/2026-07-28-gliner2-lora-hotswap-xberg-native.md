# Implementation Plan: GLiNER2 LoRA Hot-Swap + xberg-native Crate

**Spec References:**
- `docs/superpowers/specs/2026-07-27-hacienda-design.md` (hacienda architecture)
- `docs/superpowers/specs/2026-07-27-vertical-ner-architecture-design.md` (vertical NER)
- `xberg/docs/superpowers/plans/01-core-pipeline.md` (PII ecosystem crates)
- `xberg/docs/superpowers/plans/04-facade-integration.md` (xberg facade)

**Status:** Ready for Implementation  
**Priority:** Must Have (v0.2.0) — Differentiable "Verticals" Feature

---

## 📋 Executive Summary

This plan implements **runtime LoRA adapter hot-swapping** for GLiNER2 via Candle, enabling hacienda's "Verticals" feature (Finance/Healthcare/Legal PII models on shared base). Two paths:

| Path | Approach | xberg Changes | RAM | Concurrency | Timeline |
|------|----------|---------------|-----|-------------|----------|
| **Option A** | Multi-adapter cache (separate backend per adapter) | **Zero** | N × base | ✅ All loaded | 6.5 days |
| **Option B** | Trait-level hot-swap (single backend, swap weights) | ~50 lines (trait + impl) | 1 × base | ❌ Sequential | 8 days + release |

**Recommendation:** Ship **Option A immediately** (zero upstream deps, matches verticals use case), add **Option B in xberg v0.XX** for memory-constrained/WASM scenarios.

**Deliverable:** `xberg-native` crate — unified facade for hacienda without patches.

---

## 🏗️ Architecture Context

### Current State

```
hacienda-engine (this repo)
├── hacienda-core/
│   ├── src/pii/
│   │   ├── pipeline.rs          → wraps pii_pipeline::PiiPipeline
│   │   ├── config.rs            → PipelineConfig, ModelConfig
│   │   └── xberg_integration.rs → PostProcessor using pii_pipeline
│   └── Cargo.toml               → deps: pii-pipeline, pii-fastino (stub)
│
xberg (upstream)
├── crates/xberg-gliner/
│   └── src/candle/
│       ├── lora.rs              ✅ PEFT LoRA load + merge-at-load
│       ├── model.rs             ✅ Gliner2Candle::load_adapter/unload_adapter
│       └── mod.rs
├── crates/xberg/src/text/ner/candle.rs
│   └── CandleBackend            ✅ get_or_init(model_dir, adapter_dir) cache
│       cache key: (model_dir, Option<adapter_dir>)
└── crates/xberg-pii-ecosystem/
    └── pii-fastino/             ❌ Stub only (returns empty vec)
```

### Target State (Option A)

```
hacienda-engine
├── hacienda-core/
│   ├── src/pii/
│   │   ├── adapter_registry.rs  ✨ NEW — multi-adapter cache + hot-swap API
│   │   ├── pipeline.rs          ✨ MODIFIED — uses AdapterRegistry + xberg NER
│   │   ├── config.rs            ✨ MODIFIED — + lora_adapters field
│   │   ├── xberg_integration.rs ✨ MODIFIED — uses xberg NER backend
│   │   └── mod.rs               ✨ MODIFIED — exports
│   └── Cargo.toml               ✨ MODIFIED — +xberg["ner-candle"], -pii-fastino
│
├── hacienda/
│   ├── src/api.rs               ✨ MODIFIED — /pii/adapters/* endpoints
│   └── src/cli.rs               ✨ MODIFIED — hacienda pii adapters *
│
xberg (unchanged)
└── xberg-native/                ✨ NEW CRATE — unified facade
```

---

## 📦 Option A: Multi-Adapter Cache (Zero xberg Patches)

### Phase 1: hacienda-core — Adapter Registry & Config (Days 1-3)

#### 1.1 Create: `hacienda-core/src/pii/adapter_registry.rs` (~180 lines)

```rust
// Multi-adapter registry using xberg's CandleBackend::get_or_init
// Cache key: (base_model_dir, adapter_dir) → separate backend per adapter
// Supports: list, activate, deactivate, detect (uses active adapter)
```

**Key APIs:**
```rust
pub struct AdapterRegistry {
    pub fn new(base_model_dir: PathBuf, configs: Vec<AdapterConfig>) -> Result<Self>
    pub async fn get_backend(&mut self, adapter_id: &AdapterId) -> Result<Arc<CandleBackend>>
    pub async fn active_backend(&mut self) -> Result<Arc<CandleBackend>>
    pub async fn detect(&mut self, text: &str, categories: Option<&[String]>) -> Result<Vec<Entity>>
    pub fn set_active(&mut self, adapter_id: Option<AdapterId>) -> Result<()>
    pub fn list_adapters(&self) -> Vec<&AdapterConfig>
}
```

#### 1.2 Modify: `hacienda-core/src/pii/config.rs`

```rust
// Add to ModelConfig:
pub lora_adapters: Option<Vec<LoraAdapterConfig>>,

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoraAdapterConfig {
    pub id: String,
    pub name: String,
    pub path: String,
    pub categories: Vec<String>,
}
```

#### 1.3 Modify: `hacienda-core/src/pii/pipeline.rs`

```rust
pub struct PiiPipelineWrapper {
    inner: Arc<PiiPipeline>,        // regex + redaction (existing)
    adapter_registry: Option<AdapterRegistry>,
    use_ner: bool,
}

impl PiiPipelineWrapper {
    pub async fn process(&self, text: &str) -> Result<PipelineResult>
    pub fn set_active_adapter(&self, adapter_id: Option<String>) -> Result<()>
    pub fn list_adapters(&self) -> Vec<AdapterConfig>
}
```

#### 1.4 Modify: `hacienda-core/src/pii/xberg_integration.rs`

```rust
// Replace pii_pipeline::PiiPipeline with direct xberg NER usage
use xberg::text::ner::{NerBackend, candle::CandleBackend};
// Remove pii-fastino dependency
```

#### 1.5 Update: `hacienda-core/Cargo.toml`

```toml
[dependencies]
xberg = { workspace = true, features = ["ner-candle", "redaction", "ner"] }
pii-redaction = { workspace = true }
pii-regex = { workspace = true }
# REMOVE: pii-pipeline, pii-fastino, pii-config
```

---

### Phase 2: hacienda — API & CLI (Day 4)

#### 2.1 Modify: `hacienda/src/api.rs`

```rust
// GET    /v1/pii/adapters           → list + active
// POST   /v1/pii/adapters/{id}/activate
// POST   /v1/pii/adapters/deactivate
// GET    /v1/pii/adapters/{id}/status
```

#### 2.2 Modify: `hacienda/src/cli.rs`

```rust
// hacienda pii adapters list
// hacienda pii adapters activate <id>
// hacienda pii adapters deactivate
// hacienda pii adapters status [id]
```

---

### Phase 3: Config & Documentation (Day 5)

#### 3.1 Create: `config/examples/pii-with-lora.toml`

```toml
[pii.model]
enabled = true
model_id = "/models/gliner2-pii-base"
max_seq_len = 512

[[pii.model.lora_adapters]]
id = "pii-finance"
name = "Finance PII"
path = "/models/adapters/pii-finance"
categories = ["credit_card", "iban", "swift", "account_number", "routing_number"]

[[pii.model.lora_adapters]]
id = "pii-healthcare"
name = "Healthcare PII"
path = "/models/adapters/pii-healthcare"
categories = ["patient_id", "mrn", "npi", "dea_number", "icd10_code"]

[[pii.model.lora_adapters]]
id = "pii-legal"
name = "Legal PII"
path = "/models/adapters/pii-legal"
categories = ["case_number", "docket_number", "bar_number", "patent_number"]
```

#### 3.2 Create: `docs/pii-lora-adapters.md`

- Architecture diagram
- Config reference
- API/CLI reference
- Memory sizing guide (3 adapters ≈ 3 × base_model_size)
- Adapter training guide (link to GLiNER2 LoRA training)

---

### Phase 4: Testing (Day 6)

#### 4.1 Unit Tests: `hacienda-core/src/pii/adapter_registry.rs`

```rust
#[tokio::test]
async fn registry_loads_base_and_adapters() { ... }
#[tokio::test]
async fn registry_switches_active_adapter() { ... }
#[tokio::test]
async fn registry_concurrent_adapters_work() { ... }
#[tokio::test]
async fn registry_errors_on_missing_adapter_dir() { ... }
```

#### 4.2 Integration Test: `hacienda-core/tests/pii_lora_integration.rs`

```rust
#[tokio::test]
async fn full_pipeline_with_lora_adapters() {
    // 1. Start with base model
    // 2. Activate finance adapter → detect credit_card
    // 3. Switch to healthcare adapter → detect patient_id
    // 4. Deactivate → verify base model works
}
```

#### 4.3 Benchmarks: `hacienda-engine/benches/pii_lora_switch.rs`

```rust
// Cold load vs cached vs switch latency
// RAM with 1/3/5 adapters
```

---

### Phase 5: CI/CD (Day 6.5)

#### 5.1 GitHub Actions: `.github/workflows/pii-lora.yml`

```yaml
- Test with mock adapters (tiny safetensors fixtures)
- Test config parsing with lora_adapters
- Test API endpoints
- Benchmark memory usage
```

---

## 📦 Option B: Trait-Level Hot-Swap (Minimal xberg Patch)

### Phase 1: xberg — Trait Extension (Days 1-2)

#### 1.1 Modify: `xberg/crates/xberg/src/text/ner/mod.rs`

```rust
#[async_trait]
pub trait NerBackend: Send + Sync {
    async fn detect(&self, text: &str, categories: &[EntityCategory]) -> Result<Vec<Entity>>;
    
    // NEW: Optional hot-swap (default = not supported)
    async fn load_adapter(&self, _name: &str, _adapter_dir: &Path) -> Result<()> {
        Err(XbergError::Plugin { ... })
    }
    async fn unload_adapter(&self) -> Result<()> { ... }
    fn active_adapter(&self) -> Option<String> { None }
}
```

#### 1.2 Modify: `xberg/crates/xberg/src/text/ner/candle.rs`

```rust
impl CandleBackend {
    #[cfg(not(target_arch = "wasm32"))]
    pub async fn load_adapter(&self, name: &str, adapter_dir: &Path) -> Result<()> { ... }
    
    #[cfg(not(target_arch = "wasm32"))]
    pub async fn unload_adapter(&self) -> Result<()> { ... }
    
    pub fn active_adapter(&self) -> Option<String> { ... }
}

// Implement NerBackend trait methods
```

#### 1.3 Add Cache Invalidation

```rust
#[cfg(all(not(target_arch = "wasm32"), feature = "ner-candle"))]
pub fn invalidate_cache(model_dir: &Path, lora_adapter_dir: Option<&Path>) { ... }
```

---

### Phase 2: xberg — In-Memory Adapter Loading (WASM) (Day 3)

#### 2.1 Modify: `xberg/crates/xberg-gliner/src/candle/model.rs`

```rust
impl Gliner2Candle {
    pub fn from_bytes_with_adapter(
        safetensors: &[u8],
        tokenizer_json: &[u8],
        encoder_config_json: &[u8],
        adapter_config_json: &[u8],
        adapter_safetensors: &[u8],
    ) -> Result<Self> { ... }
}
```

#### 2.2 Modify: `xberg/crates/xberg/src/text/ner/candle.rs`

```rust
impl CandleBackend {
    pub fn from_bytes_with_adapter(...) -> Result<Self> { ... }
}
```

---

### Phase 3: hacienda-core — Single-Backend Hot-Swap (Day 4)

```rust
pub struct PiiPipelineWrapper {
    ner_backend: Option<Arc<dyn NerBackend>>,  // Single shared backend
}

impl PiiPipelineWrapper {
    pub async fn hot_swap_adapter(&self, name: &str, dir: &Path) -> Result<()> { ... }
    pub async fn unload_adapter(&self) -> Result<()> { ... }
}
```

---

### Phase 4: xberg Release + hacienda Migration (Days 5-8)

- xberg: PR, review, publish (semver MINOR — trait has defaults)
- hacienda: Swap impl, test, release

---

## 📦 xberg-native Crate (Deliverable)

### Purpose
Unified facade for hacienda — **single import, zero patches to xberg**.

### Location: `crates/xberg-native/` (new crate in hacienda-engine workspace)

```toml
# crates/xberg-native/Cargo.toml
[package]
name = "xberg-native"
version = "0.1.0"
edition = "2021"
description = "Unified xberg facade for hacienda — extract + PII + verticals"

[dependencies]
xberg = { workspace = true, features = ["ner-candle", "redaction", "ner", "pii"] }
hacienda-core = { path = "../hacienda-core" }

[features]
default = ["pii-verticals"]
pii-verticals = ["hacienda-core/pii"]
```

```rust
// crates/xberg-native/src/lib.rs
pub use xberg::*;
pub use hacienda_core::pii::{PiiPipelineWrapper, PipelineConfig, AdapterRegistry, AdapterConfig};
pub use hacienda_core::facade::HaciendaFacade;
pub use hacienda_core::config::HaciendaFacadeConfig;

/// One-liner setup for hacienda apps
pub struct XbergNative {
    pub facade: HaciendaFacade,
    pub pii: Option<PiiPipelineWrapper>,
}

impl XbergNative {
    pub fn new(config: HaciendaFacadeConfig) -> Result<Self, HaciendaError> {
        let facade = HaciendaFacade::new(config.clone())?;
        let pii = config.pii.as_ref().map(|c| PiiPipelineWrapper::new(c.clone())).transpose()?;
        Ok(Self { facade, pii })
    }
    
    /// Extract + PII with active adapter
    pub async fn process(&self, input: ExtractInput) -> Result<HaciendaResult> {
        self.facade.process(input).await
    }
    
    /// Hot-swap LoRA adapter (Option A: Finance → Healthcare → Legal)
    pub fn set_adapter(&self, adapter_id: Option<String>) -> Result<()> {
        self.pii.as_ref().map(|p| p.set_active_adapter(adapter_id)).unwrap_or(Ok(()))
    }
}
```

### hacienda Usage

```rust
// Before: Multiple imports, manual wiring
use xberg::*;
use hacienda_core::*;

// After: Single import
use xberg_native::XbergNative;

let native = XbergNative::new(HaciendaFacadeConfig {
    extraction: Default::default(),
    pii: Some(PipelineConfig {
        model: ModelConfig {
            enabled: true,
            model_id: "/models/gliner2-base".into(),
            lora_adapters: Some(vec![
                LoraAdapterConfig { id: "finance".into(), ... },
                LoraAdapterConfig { id: "healthcare".into(), ... },
            ]),
            ..
        },
        ..
    }),
    ..
})?;

// Switch vertical at runtime
native.set_adapter(Some("finance".into()))?;
let result = native.process(ExtractInput::from_text("Patient John Doe, SSN 123-45-6789")).await?;
native.set_adapter(Some("healthcare".into()))?;
let result = native.process(ExtractInput::from_text("Patient Jane Smith, MRN 987654")).await?;
```

---

## ✅ Acceptance Criteria

### Option A (Ship First)

- [ ] `hacienda pii adapters list` shows all configured adapters
- [ ] `hacienda pii adapters activate finance` switches active adapter
- [ ] Finance adapter detects `credit_card`, `iban`, `swift`
- [ ] Healthcare adapter detects `patient_id`, `mrn`, `npi`
- [ ] Legal adapter detects `case_number`, `docket_number`
- [ ] Base model (no adapter) detects `person`, `org`, `email`
- [ ] Concurrent adapters: all 3 loaded, switch < 5ms
- [ ] Memory: ~3 × base_model_size (e.g., 3 × 500MB = 1.5GB)
- [ ] Config file with `lora_adapters` parses correctly
- [ ] All existing PII tests pass (regression)

### Option B (Follow-up)

- [ ] Single backend, hot-swap < 200ms (merge time)
- [ ] RAM: 1 × base_model_size
- [ ] `from_bytes_with_adapter` works on WASM
- [ ] xberg trait published, hacienda migrated

### xberg-native

- [ ] `use xberg_native::XbergNative` compiles
- [ ] All hacienda types re-exported
- [ ] Single facade for extract + PII + verticals
- [ ] Zero patches to xberg required

---

## 📊 Effort Summary

| Option | Files | Lines | Days | Risk |
|--------|-------|-------|------|------|
| **A (Multi-Cache)** | 10 | ~900 | 6.5 | Low (zero upstream) |
| **B (Trait Hot-Swap)** | 10 | ~580 | 8 + release | Medium (trait change) |
| **xberg-native** | 1 | ~50 | 0.5 | None |

---

## 🚀 Rollout Strategy

| Week | Milestone |
|------|-----------|
| **1** | Option A complete — hacienda-engine v0.2.0 with verticals |
| **2** | xberg-native crate published, docs, examples |
| **3-4** | Option B PR to xberg (parallel, non-blocking) |
| **5** | xberg releases with trait, hacienda migrates to Option B impl |
| **6** | WASM in-memory adapter loading (Option B Phase 2) |

---

## 🔗 Related Docs

- `docs/superpowers/specs/2026-07-27-hacienda-design.md` — hacienda architecture
- `docs/superpowers/specs/2026-07-27-vertical-ner-architecture-design.md` — vertical taxonomy
- `docs/configuration.md` — PII config examples (add lora_adapters)
- `xberg/docs/superpowers/plans/04-facade-integration.md` — xberg facade pattern
- `xberg/docs/superpowers/plans/01-core-pipeline.md` — PII ecosystem crates

---

**Approval:** Ready for implementation. Option A unblocks verticals feature immediately; Option B enables future WASM/edge deployment.