# Implementation Plan: PII Candle Path — Minimal Phase 1

**Supersedes the Phase 1 section of:**

- `docs/superpowers/plans/2026-07-28-client-lora-ecosystem-deep-integration.md`
- `docs/superpowers/plans/2026-07-28-gliner2-lora-hotswap-xberg-native.md`

**Status:** Ready for implementation
**Priority:** Blocker — every downstream LoRA/registry/SaaS phase depends on this
**Scope:** hacienda-engine only. No registry, no remote fetch, no auth, no training, no Studio work.

---

## Goal

Make `hacienda pii scan` return a real, model-detected entity, with an optional local LoRA
adapter applied.

That is the whole milestone. One text in, one non-regex entity out.

## Non-goals (explicitly deferred)

| Deferred | To |
| --- | --- |
| `LoRARegistry`, `AdapterId`, `AdapterMetadata` | Phase 2, once there is a working detector to register adapters *for* |
| Remote adapter pull/push, sha256 verification, `RegistryAuth` | Phase 4 |
| `/v1/pii/adapters/*` CRUD endpoints | Phase 2 |
| MCP tools, Hacienda-mind changes | Phase 3 |
| Studio UI, WASM `from_bytes_with_adapter` | Phase 3 |
| LoRA training (does not exist in any repo today) | Phase 5 |
| `xberg-native` facade crate, `XbergNative` type | Defer until a second consumer exists |

---

## Verified ground truth

Checked 2026-07-28 against working trees, not assumed.

### What already works (upstream, in xberg)

| Capability | Location |
| --- | --- |
| PEFT LoRA load + merge-at-load | `xberg/crates/xberg-gliner/src/candle/model.rs:134` `load_adapter` |
| Adapter unload (re-reads base from disk) | `.../candle/model.rs:163` `unload_adapter` |
| Reads `adapter_config.json` + `adapter_model.safetensors`, `merge_into_base()` | `.../candle/lora.rs` |
| Base-model mismatch guard — **refuses** to merge when `base_model_name_or_path` disagrees | `.../candle/model.rs:137-147` |
| `NerBackend` impl with `block_in_place` around CPU-bound inference | `xberg/crates/xberg/src/text/ner/candle.rs:149` |
| **Multi-adapter cache already exists** | `.../ner/candle.rs:111` `CandleBackend::get_or_init(model_dir, lora_adapter_dir: Option<&Path>)`, backed by `CANDLE_BACKEND_CACHE` keyed on `(model_dir, Option<adapter_dir>)` |
| Cargo feature | `xberg/crates/xberg/Cargo.toml:372` `ner-candle` |

**Consequence:** the "Option A multi-adapter cache" described in the ecosystem plan is
already implemented upstream. hacienda-engine does not need to build it. It needs to *call*
`get_or_init` with the right `Option<&Path>`.

### What is broken (here, in hacienda-engine's dependency chain)

The ML detection path is dead in two places:

```rust
// xberg-pii-ecosystem/crates/pii-pipeline/src/pipeline.rs:56
let model_entities = Vec::new();          // _model_backend is constructed, then never called
```

```rust
// xberg-pii-ecosystem/crates/pii-fastino/src/loader.rs:16
pub fn extract(&self, _text, _labels, _threshold) -> Result<Vec<ExtractedEntity>, String> {
    Ok(vec![])                            // stub: unconditionally empty
}
```

`pii_fastino::LoraAdapter` (`lora.rs`) has no `load()` and no merge — it is a type sketch.

So today `hacienda pii scan` runs **regex only**. `ModelConfig.enabled` has no observable
effect. Any claim that the LoRA foundation "exists" in hacienda-engine is false; it exists
in xberg.

### Pre-existing compile breakage in `hacienda-core/src/pii/`

Fix before anything else — this module does not appear to be in a buildable state:

1. `pipeline.rs:1-2` reference the crate by its own name
   (`use hacienda_core::pii::profiles::RedactionProfile;`) instead of `crate::`.
2. `mod profiles;` is not declared in `pii/mod.rs`, so `profiles.rs` is orphaned.
3. `PiiPipelineWrapper` is defined **twice** — `mod.rs` (`inner: PiiPipeline`) and
   `pipeline.rs` (`inner: Arc<PiiPipeline>`) — while `mod.rs` does `pub use pipeline::*;`.
4. `ModelConfig` is duplicated across `config.rs:54` and `pii/config.rs:28`.

Note: `cargo check` currently fails at manifest resolution before reaching this code —
`xberg/Cargo.toml` requires the unstable `codegen-backend` feature, which stable cargo
1.97.1 rejects. Resolve the toolchain question (pin a `rust-toolchain.toml`, or drop the
feature upstream) as step 0, otherwise none of the below is verifiable.

---

## Steps

Each step ends in something observable. Do not batch them.

### Step 0 — Make the workspace build

- Decide the toolchain: pin `rust-toolchain.toml` to the nightly xberg needs, or remove
  `codegen-backend` from `xberg/Cargo.toml`.
- **Verify:** `cargo check -p hacienda-core` resolves manifests and reports only real
  type errors.

### Step 1 — Repair `hacienda-core/src/pii/`

- Delete the duplicate `PiiPipelineWrapper` in `mod.rs`; keep the `pipeline.rs` one.
- Declare `mod profiles;` in `pii/mod.rs`.
- Replace self-referential `hacienda_core::` paths with `crate::`.
- Collapse the duplicate `ModelConfig` to one definition.
- **Verify:** `cargo check -p hacienda-core` is clean. No behavior change yet.

### Step 2 — Add a regression test that proves detection is dead

Before fixing anything, encode the bug:

```rust
#[tokio::test]
async fn model_backend_detects_a_name_that_regex_cannot() {
    // "Contact Marie Dubois about the account" — no regex pattern matches a person name
    let result = wrapper.process("Contact Marie Dubois about the account").unwrap();
    assert!(
        result.entities.iter().any(|e| e.category == "PERSON"),
        "model path returned no PERSON entity: {:?}", result.entities
    );
}
```

- **Verify:** this test **fails** today. That failure is the milestone's definition of done,
  inverted. Do not proceed until you have watched it fail for the right reason.

### Step 3 — Route detection through xberg's Candle backend

Replace the `pii-fastino` call site. Do **not** modify `pii-pipeline` upstream; instead have
`PiiPipelineWrapper` own the NER call and merge, so hacienda-engine controls the seam.

- `hacienda-core/Cargo.toml`: `xberg = { workspace = true, features = ["ner", "redaction", "ner-candle"] }`;
  drop the `pii-fastino` dependency.
- In `PiiPipelineWrapper`:
  - hold `Option<Arc<CandleBackend>>` obtained via
    `CandleBackend::get_or_init(&model_dir, lora_adapter_dir.as_deref())`,
  - call `NerBackend::detect(text, &categories)` for the model entities,
  - merge model entities with the existing regex entities via the existing merge config.

**`process()` must become `async fn`.** `NerBackend::detect` is async and today
`PiiPipelineWrapper::process(&self, text)` is synchronous. This ripples to `hacienda/src/api.rs`
(handlers are already async — fine) and `hacienda/src/cli.rs` (needs an await). This signature
change is the real integration cost and is why it belongs in Phase 1 rather than being
discovered in Phase 2.

- **Verify:** the Step 2 test passes. Regex-only behavior for e.g. email is unchanged.

### Step 4 — Config: one local adapter, from disk

Extend `ModelConfig` (single definition, post-Step-1):

```toml
[pii.model]
enabled      = true
model_id     = "fastino/GLiNER2-Guardrails-PII-Multi"   # current default, config.rs:69
model_dir    = "~/.cache/hacienda/models/gliner2-guardrails-pii"
adapter_dir  = "./adapters/pii-finance"                  # optional; None = base model
```

Naming note: the ecosystem plan's `gliner2-pii-base-v1` does not exist. The real identifiers
are `fastino/GLiNER2-Guardrails-PII-Multi` (`hacienda-core/src/config.rs:69`) and the Studio
IndexedDB keys `gliner2-guardrails-pii-{model,tokenizer,config}`
(`apps/hacienda-studio/lib/asset-loader.ts:30-35`). Use the real names — `load_adapter`
enforces `base_model_name_or_path` agreement and will hard-refuse a mismatch.

- Path expansion for `~` and relative paths, resolved once at construction.
- Fail fast with a clear message if `adapter_dir` is set but missing, rather than silently
  falling back to base.
- **Verify:** a test asserting `adapter_dir = None` and `adapter_dir = Some(path)` produce
  two distinct `Arc<CandleBackend>` instances from the cache, and that a bogus `adapter_dir`
  errors at construction.

### Step 5 — Surface it

- CLI: add `--adapter <path>` to `PiiCommands::Scan` and `Redact` (`hacienda/src/cli.rs:32`),
  overriding config. Local paths only.
- API: accept an optional `adapter` field on the existing `/v1/pii/scan` and `/v1/pii/redact`
  bodies (`hacienda/src/api.rs:15-16`). No new routes.
  - **Security:** the API-supplied value must be resolved against an allowlist of configured
    adapter directories, never used as a raw filesystem path from the request body. An
    unrestricted path here is arbitrary-file-read plus arbitrary-weight-load.
- **Verify:** `hacienda pii scan "Contact Marie Dubois" --adapter ./adapters/pii-finance`
  returns entities, and differs from the no-adapter run on at least one fixture.

---

## Exit criteria

- [ ] `cargo check` and `cargo test -p hacienda-core` pass on a pinned toolchain.
- [ ] Step 2's test passes: a PERSON entity no regex can find is returned by the model path.
- [ ] Base vs. adapter produce measurably different output on a fixture.
- [ ] A missing or mismatched adapter fails loudly at construction, not silently at inference.
- [ ] `pii-fastino` is no longer a dependency of `hacienda-core`.
- [ ] Adapter switch latency is **measured** and recorded — do not carry forward the
      ecosystem plan's unbenchmarked "<1ms / <200ms" figures. `merge_into_base` re-reads the
      full `model.safetensors` and rebuilds encoder + heads; `unload_adapter` re-reads from
      disk again. Publish real numbers or none.

## What this unblocks

Once adapters demonstrably change output, Phase 2 (registry) has something to register and a
metric to justify itself. Until then, a registry, a training wizard, and a multi-region GPU
service are all abstractions over `Ok(vec![])`.
