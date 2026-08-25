# Moving `xberg` to Upstream Latest: Feasibility and wasm Impact

**Date:** 2026-08-24
**Status:** Investigation complete — recommendation pending decision
**Relates to:** `Cargo.toml:214-227` (the temporary fork exception),
`docs/architecture/README.md` §3 (policy against the fork entering the dependency graph)
**Companion:** `2026-08-24-studio-knowledge-base-export-design.md` (§2.2–§2.4's constraints are
re-verified against upstream latest here)

---

## 1. Question

`Cargo.toml:228` pins `xberg` to `github.com/jamon8888/xberg.git@9bfbc10`, documented as a
**temporary** exception:

> jamon8888/xberg@fix/test-documents-submodule-pin is otherwise the identical tree to
> xberg-io/xberg's v1.0.2 tag (commit 9dcc864d) with only that one gitlink repointed […]
> revert to the upstream tag once xberg-io/xberg or xberg-io/test_documents fixes the submodule
> pin upstream.

Two questions: **can we revert now**, and **what does it do to hacienda-wasm**?

---

## 2. Measured Ground Truth

All checks run 2026-08-24 against live remotes.

### 2.1 The blocker is fixed — but only on `main`, not on any tag

Cargo fetches a git dependency's submodules unconditionally during resolution, so an unfetchable
`test_documents` gitlink breaks `cargo build` for the whole workspace. Each tag carries a
*different* gitlink, so the existing claim ("true of every tag from v1.0.2 through v1.0.14") was
worth testing rather than assuming. Tested by direct `git fetch <sha>` against
`xberg-io/test_documents`:

| Ref | `test_documents` gitlink | Fetchable? |
| --- | --- | --- |
| `v1.0.2` (our pinned tree) | `4ca54bb0` | **BROKEN** |
| `v1.0.12` | `3b58c72b` | **BROKEN** |
| `v1.0.14` (latest tag) | `1112215e` | **BROKEN** |
| `main` (`aa47d873`, 2026-08-24) | `44f2c9fe` | **FETCHABLE** |

The claim holds and now extends through the latest tag. **There is no tagged release of xberg that
this workspace can build against.** The only escape from the fork is upstream `main`.

### 2.2 Size of the move

| | Ours | Upstream `main` |
| --- | --- | --- |
| Version | 1.0.2 | 1.1.0 (unreleased) |
| Commits | — | **1126** ahead of the v1.0.2 tree |
| Latest tag | — | v1.0.14 (2026-08-04) |

### 2.3 The two BREAKING changes in 1.1.0 — neither affects us

Both sit inside the `## [1.1.0] - Unreleased` section (CHANGELOG lines 10–1253), i.e. they are on
`main` and in no tagged release.

**`ExtractedDocument.formatted_content` dropped from bindings** (CHANGELOG:362). The entry is
explicit that this is a *bindings-only* removal: "The field remains `pub` on the Rust type, where
extractor and post-processor plugins legitimately use it." hacienda touches it only from Rust —
`hacienda-core/src/facade.rs:1690` mutates `document.formatted_content` in a post-processing step.
**Unaffected.**

**PDF backend renamed `pdf_oxide` → `native`** (CHANGELOG:227), with no back-compat alias — the old
value is rejected with an error. hacienda's only references to `pdf_oxide` are *prose in comments*
(`apps/hacienda-studio/lib/pdf-liteparse.ts:3`, `lib/types.ts:150`). No PDF backend string is set
anywhere at runtime; `worker/pipeline.ts:636-640` configures chunking and OCR only, and the entry
notes "the default is unchanged". **No runtime impact; two stale comments to correct.**

### 2.4 One behavioural change that does land — on RAG, not on wasm

**Chunk `content` no longer prepends the heading breadcrumb** (CHANGELOG:295). Chunks now contain
exactly the source span they were cut from, so `content.len()` agrees with
`byte_end - byte_start`.

Studio sets `WasmChunkingConfig` (`worker/pipeline.ts:636-637`) but never reads per-chunk content —
it consumes `result.results[0].content`, the whole document. The consumers are native:
`crates/hacienda-rag/src/chunk.rs:34` and `src/stream.rs:175-227`.

This is a **retrieval-quality change, not a compile break**, and arguably an improvement for
hacienda-rag's lexical paths for the reason the changelog gives (breadcrumb repetition collapsing
IDF). It needs validating, not fixing.

### 2.5 The wasm surface is unchanged

Every symbol `worker/pipeline.ts` imports from `@xberg-io/xberg-wasm` still exists on upstream
`main`: `WasmExtractInput`, `WasmExtractionConfig`, `WasmOutputFormat`, `WasmChunkingConfig`,
`WasmOcrConfig`, `WasmNerConfig`, `WasmNerBackendKind`, `WasmImageExtractionConfig`,
`WasmSecurityLimits`, `XbergEngine`, `NerModel` — 11 of 11.

`NerModel.detect(text, opts)` is byte-for-byte the same shape
(`crates/xberg-wasm/src/bridge/ner_model.rs:115-116` on main): still `categories_from_opts`, still
no threshold parameter.

### 2.6 Upgrading does **not** unlock the KB spec's Tier 2

Re-verified against upstream `main`, because it would change
`2026-08-24-studio-knowledge-base-export-design.md`'s roadmap if it had moved:

- `crates/xberg-gliner/src/candle/heads/mod.rs` still reads `MAX_WIDTH: usize = 8`.
- The `classifier` head is still "intentionally NOT ported; this crate ships `extract_ner` parity
  only" — same comment, verbatim.
- No relation or structured-extraction head exists.

**§2.2, §2.3 and §2.4 of that spec hold on latest upstream.** Tier 2 remains an upstream porting
project; a version bump does not deliver it. *(Note: the `xberg-gliner` → `xberg-gliner-candle`
crate split observed earlier is a change on the **fork's** divergent `main`, not upstream's. On
upstream the path is unchanged.)*

### 2.7 The vendor patches survive the move

`vendor/candle-transformers-0.11.0-wasm-fix` patches debertav2's hardcoded-f32 scalars, which break
every F16 forward pass (`Cargo.toml:349-367`). Still required:

- Upstream `main` still declares `candle-transformers = { version = "0.11" }` — the patch still
  applies cleanly against the same major.
- Upstream's `crates/xberg-gliner/src/candle/encoder.rs:54-65` still takes a caller-supplied `dtype`
  and passes `DType::F16` through to `VarBuilder::from_tensors`, so the F16 path — and therefore the
  bug — is still live. Nothing upstream fixed it.

`vendor/candle-core-0.11.0-wasm-fix` is likewise unaffected (same candle major).

### 2.8 Pre-existing version skew, worth recording

The vendored npm package is `@xberg-io/xberg-wasm@1.0.12-hacienda-f16fix.0`
(`vendor/xberg-wasm-ner-fix/package.json`), while the Rust dependency is pinned to the **v1.0.2**
tree. The product currently ships two different xberg versions on two surfaces. This predates the
question but is the kind of skew an upgrade should resolve rather than perpetuate.

---

## 3. Assessment

The fork exists for exactly one reason — a broken submodule gitlink — and that reason is now
resolved upstream, but only on an unreleased branch. So the choice is:

| Option | Viable? | Cost |
| --- | --- | --- |
| Stay on the fork | yes | Keeps a documented architecture-policy exception open indefinitely |
| Move to a tag (≤ v1.0.14) | **no** | Every tag's submodule gitlink is unfetchable (§2.1) |
| Move to upstream `main` @ pinned SHA | yes | 1126 commits of drift; one RAG behaviour change to validate |

**Recommendation: move to upstream `main` pinned to a specific SHA** (e.g. `aa47d873`), not to the
branch name. That closes the `Cargo.toml:214-227` exception and the
`docs/architecture/README.md` §3 violation, and the measured blast radius on the wasm side is zero:
no symbol removed, no signature changed, both vendor patches still needed and still applicable.

The honest caveat is that `main` is an unreleased moving target. Pinning a SHA makes it no less
reproducible than today's fork pin — which is also a SHA on a branch — but it does mean the next
bump needs the same review. The durable fix is upstream tagging a release whose submodule pin
resolves; that is worth an upstream issue, since §2.1 shows the breakage has persisted across at
least 13 consecutive tags.

---

## 4. Verification Before Merging

1. `cargo build` for the workspace, and `cargo build -p hacienda-wasm --features ner-candle-wasm`
   for the wasm target — confirms the submodule fetch and both vendor patches resolve.
2. `task build:wasm`, then the `ci-wasm-freshness` check — the committed `pkg/` must match a rebuild
   (`.github/workflows/ci-wasm-freshness.yaml:90`).
3. Studio e2e: `pipeline.spec.ts`, `pseudonymize.spec.ts`, `folder-upload.spec.ts` — exercises the
   full extract → NER → PII → export path across the 11 wasm symbols.
4. **Neural NER must still load.** `worker/pipeline.ts:150` logs "Neural NER backend loaded"; a
   regression in the candle-transformers patch shows up as the silent regex fallback at `:158`, not
   as an error. Assert on the log line, not on output shape.
5. hacienda-rag retrieval quality, given §2.4's chunk-content change — the one place behaviour
   genuinely moves.
6. Decide whether to rebuild `vendor/xberg-wasm-ner-fix` from the same SHA, closing §2.8's skew.

---

## 5. Open Questions

1. **Should the npm vendor rebuild be part of this change or a follow-up?** Doing both at once makes
   a regression harder to bisect; doing them separately leaves §2.8's skew open for longer.
2. **Is an upstream issue worth filing for the submodule pin?** Thirteen consecutive tags ship
   unbuildable-as-a-git-dependency. Other consumers presumably hit this; a fix upstream removes the
   need for any SHA pinning here.
3. **Does hacienda-rag have a retrieval regression test?** §2.4's change is invisible to compilation
   and to Studio. Without one, the breadcrumb removal lands unmeasured.
