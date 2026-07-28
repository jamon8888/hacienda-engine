# Findings: PII Candle Phase 1 — Scope Is Larger Than Planned

**Companion to:** `2026-07-28-pii-candle-phase1-minimal.md`
**Status:** Investigation only — no source changes made in `hacienda-core`/`hacienda` this pass.

## Why this doc exists

The Phase 1 plan's "Step 1 — Repair `hacienda-core/src/pii/`" lists four fixes and treats
them as the only blocker before wiring the Candle backend. On inspection, the breakage is
substantially wider than that: the `hacienda-core` and `hacienda` crates are internally
inconsistent across most of their public surface, not just in `pii/`. Attempting the plan's
Step 1 in isolation would not get `hacienda-core` to compile.

**Also:** this sandbox has no access to `xberg` or `xberg-pii-ecosystem` — the workspace's
path dependencies. `cargo check` fails at manifest resolution before reaching any Rust source:

```
error: failed to load manifest for workspace member `hacienda`
Caused by: failed to read `/home/user/xberg-pii-ecosystem/crates/pii-audit/Cargo.toml`
Caused by: No such file or directory (os error 2)
```

Everything below was found by reading source, not by compiling. Treat it as leads, not
verified diagnoses — the real `pii_redaction`/`pii_pipeline`/`pii_config` crate shapes are
unknown here and confirming any fix needs a working build.

## Confirmed: the plan's four Step 1 items are real

1. `pii/pipeline.rs:1-2` — self-referential `hacienda_core::` imports (should be `crate::`).
2. `pii/mod.rs` doesn't declare `mod profiles;` — `pii/profiles.rs` is orphaned.
3. `PiiPipelineWrapper` is defined twice: `pii/mod.rs:12` (`inner: PiiPipeline`) and
   `pii/pipeline.rs:9` (`inner: Arc<PiiPipeline>, config: PipelineConfig`). `mod.rs` also
   does `pub use pipeline::*;`, so even after removing one definition there's a second,
   separate collision (see below).
4. `ModelConfig` (and `PipelineConfig`, `RedactionProfile`, `CustomProfile`) are duplicated
   between `hacienda-core/src/config.rs` (`pub mod pii { ... }`) and `hacienda-core/src/pii/config.rs`.

## New: bugs outside the plan's Step 1 scope

### `pii/mod.rs` vs `pii/pipeline.rs` — a second duplicate the plan didn't list

`mod.rs` locally defines `PipelineResult`, `PipelineEntity`, `PipelineAuditEntry`,
`PipelineMetrics` (lines 29-61) with a `From<pii_pipeline::PipelineResult>` conversion.
`pipeline.rs` separately does `pub use pii_pipeline::{PipelineAuditEntry, PipelineEntity,
PipelineMetrics, PipelineResult};` and `mod.rs` glob-imports that via `pub use pipeline::*;`.
That's four more name collisions in the same module, on top of `PiiPipelineWrapper`.
Deleting "the duplicate `PiiPipelineWrapper`" per the plan doesn't resolve these — they need
their own decision about which shape is canonical (mod.rs's converted wrapper types, or
pipeline.rs's raw pass-through of the external crate's types).

### `hacienda-core/src/redaction/mod.rs` is broken independent of `pii/`

- Declares `pub mod engine; pub mod fpe; pub mod patterns;` — **none of the three files
  exist.** Hard compile error (`E0583`), unrelated to anything in the Phase 1 plan.
- Imports `RedactionConfig`, `RedactionMode`, `RedactionResult` from `pii_redaction`
  **and** redefines all three locally in the same file (lines 23-43) — a duplicate-name
  compile error independent of the `pii/` module.
- The local `RedactionConfig` (fields: `mode`, `fpe_key`, `custom_template`,
  `preserve_format`) doesn't match how its only consumer, `pii/profiles.rs`, constructs it
  (`custom_patterns: vec![...]` — different field, different type, plural). So even isolated
  from the `pii_redaction` collision, `profiles.rs` doesn't compile against the local struct
  it's supposedly written for. There's no way to tell from this repo alone which shape is
  intended — that answer lives in `pii_redaction`'s real definition, which isn't accessible
  here.

### `hacienda-core/src/facade.rs` references a type that doesn't exist

`use hacienda_core::pii::{PiiPipeline, PipelineConfig, PipelineResult};` (line 4) and again
in a local `mod pii { ... }` block (line 207). `hacienda_core::pii` has no `PiiPipeline` —
only `PiiPipelineWrapper`. This is the actual production entry point (`HaciendaFacade::new`/
`process`), so this isn't a dead path; it's load-bearing and still broken.

### `hacienda/src/lib.rs` re-exports ~15 items that don't exist in `hacienda-core`

```rust
pub use hacienda_core::{
    PiiPipeline, PipelineConfig, PipelineResult, PipelineEntity,
    PipelineAuditEntry, PipelineMetrics,
    RedactionMode, RedactionConfig as HaciendaRedactionConfig,
    RedactionProfile, PciProfile, HipaaProfile, CustomProfile,
    ...
};
```

None of `PiiPipeline`, `RedactionMode`, `RedactionConfig`, `RedactionProfile` are exported at
the `hacienda_core` crate root (`lib.rs` only does `pub mod pii;` / `pub mod redaction;`, not
`pub use pii::*;` / `pub use redaction::*;` at the root). `PciProfile` and `HipaaProfile`
aren't defined as distinct types anywhere in the codebase at all — the closest thing is
`RedactionProfileImpl::pci()` / `::hipaa()` constructor methods on a single type in the
(currently undeclared) `pii/profiles.rs`.

## What this means for Phase 1

The plan's premise — "fix four things in `pii/`, then wire the Candle backend" — undersells
the real state. `hacienda-core` and `hacienda` are not two crates with one broken corner;
they're two crates whose public APIs were written against each other's *intended* shapes,
not their *actual* shapes, in multiple unrelated places (`pii/`, `redaction/`, `facade.rs`,
`lib.rs`). None of this is reachable or fixable purely by reading `pii/` in isolation as
Step 1 assumes.

## Recommendation

Before resuming Step 1:

1. Get a real `cargo check` working — either by making `xberg` and `xberg-pii-ecosystem`
   available to whatever session does this work, or by temporarily stubbing them out enough
   to isolate `hacienda-core`'s own internal consistency from the external crates.
2. Re-scope Step 1 to cover the actual blocker set: `pii/mod.rs`, `pii/pipeline.rs`,
   `pii/profiles.rs`, `pii/config.rs`, `config.rs`, `redaction/mod.rs`, `facade.rs`, and
   `hacienda/src/lib.rs` — not just `pii/`.
3. Treat `redaction/mod.rs`'s missing `engine.rs`/`fpe.rs`/`patterns.rs` and the
   `RedactionConfig` shape question as their own decision: either those files need to be
   written, or the `pub mod` declarations and the local duplicate types need to be deleted
   in favor of `pii_redaction`'s real types — that choice depends on whether hacienda-core's
   own redaction module is meant to be load-bearing or the external crate is.
