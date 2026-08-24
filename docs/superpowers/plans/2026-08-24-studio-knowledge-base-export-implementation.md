# Studio Knowledge-Base Export, Tier 0 — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Stop the Studio zip export from (a) asserting relationships it never observed, (b) emptying its own entity layer in `pseudonymize` mode, and (c) renumbering every identity on re-export — then give the bundle the structure a filesystem-MCP reader can actually navigate.

**Architecture:** Everything here is inside `apps/hacienda-studio/` plus a two-line correction in `hacienda-core/src/compliance/model_card.rs`. **No model change, no new inference pass, no `xberg` version bump, no upstream dependency.** Every derived fact in this plan comes from data `BatchEntityRegistry` and `processFile` already produce. Tier 1 (zero-shot semantic labels) and Tier 2 (classifier/relation head ports) are explicitly **out of scope** — see the spec's §8 gates.

**Tech Stack:** TypeScript, existing deps only (`jszip`, `vitest` 2.1, `@playwright/test`). No new npm dependency is introduced by any task.

**Spec:** `docs/superpowers/specs/2026-08-24-studio-knowledge-base-export-design.md` §8 steps 2, 2b, 3, 4, 5, 6. Steps 1 (eval harness), 7 (Tier 1) and 9–10 (Tier 2) are **not** in this plan.

**Dependency decision:** `docs/superpowers/specs/2026-08-24-xberg-upstream-upgrade-investigation.md` recommends **not** bumping `xberg` now. Nothing in this plan depends on that decision either way — verified: all 11 wasm symbols and `NerModel.detect`'s signature are identical on our pin and on upstream `main`.

**Baseline:** Task 0 is **complete** (2026-08-24) with one item outstanding — reference bundles were
not producible on this host. See Task 0 for what that blocks.

**Known-good commands on this host** (Task 0 established the obvious ones are wrong — use these, not
the `package.json` script names):

| Purpose | Command | Note |
|---|---|---|
| Unit tests | `cd apps/hacienda-studio && npx vitest run` | **Not** `npm run test:unit` — its `pretest:unit` wasm build fails here (Task 0.2) |
| Typecheck | `cd apps/hacienda-studio && npx tsc --noEmit` | 9 pre-existing errors — compare against Task 0's list, don't expect zero |
| Compliance tests | `cargo test -p hacienda-core --lib compliance::` | **Not** `… -p hacienda-core compliance`, which matches 0 tests |
| e2e | *unavailable* | Blocked by the same wasm build (Task 0.2). Not a gate for this plan. |

---

## Ground Truth — Verified vs Assumed

Every row read from source on 2026-08-24.

**Verified by reading source:**

| Fact | Location |
|---|---|
| `inferRelationships` emits `works_for`/`partner_of`/`contact_email` from bare co-occurrence, with hardcoded confidences (0.7/0.5/0.8/0.7) and context `"Co-occurs in document"` | `lib/registry.ts:131-224` |
| It is called once per document, after entity registration | `worker/pipeline.ts:1103` |
| Relationships reach the export via `toJSON()` → `entities-registry.json`, and via `KGExporter` → all three `kg-export/` files | `lib/registry.ts:236-246`, `lib/kg-export.ts:6-58`, `lib/zip-export.ts:179,200-206` |
| `README.md` in the bundle instructs the reader to prefer the KG over the prose | `lib/zip-export.ts:135-137` |
| Entity ids are assignment-ordinal `ent-NNN` | `lib/registry.ts:81` |
| Document ids are assignment-ordinal `doc-NNN` | `worker/pipeline.ts:1111` |
| `aliases` is declared and initialised to `[]`; **nothing ever writes to it** | `lib/registry.ts:11,91` (grep: no other write) |
| `nerCategoryToPiiCategory` maps `person → "person"`, `organization → "organization"` | `worker/pipeline.ts:350-353` |
| `filterExportableEntities` drops any entity with any span overlapping any PII finding, when `redactPiiInOutput` | `worker/pipeline.ts:451-457` |
| `pseudonymize` "only takes effect when `redactPiiInOutput` is also set" | `lib/types.ts:133` |
| **Therefore in pseudonymize mode every person and organisation is dropped from `entities/`, `GLOSSARY.md`, the registry and all KG exports** | composition of the four rows above |
| Pseudonym tokens are minted at `:857`, **before** `filterExportableEntities` at `:905` — so at filter time each finding's `redact_template` already holds its token | `worker/pipeline.ts:857,905` |
| Token format is `[LABEL:keyId:base32]`, produced by AES-SIV — **deterministic**, so the same (category, text) yields the same token for the whole batch | `lib/pseudonymize.ts:428-442` |
| The batch key is derived once in `processFiles`, shared by every `processFile` call | `worker/pipeline.ts:1046-1058` |
| `entityFileName` is `${type}-${slug}.md`, slug from the **first mention's** surface form | `lib/annotate.ts:28`, `lib/registry.ts:88` |
| Frontmatter serialises entities as a single-line JSON blob | `worker/pipeline.ts:482` |
| `GLOSSARY.md` has no size gate — every entity, every type | `lib/zip-export.ts:60-88` |
| Bundle members today: `README.md`, `GLOSSARY.md`, `_manifest.json`, `entities-registry.json`, `documents/`, `entities/`, `kg-export/` | `lib/zip-export.ts:151-207` |
| **No document→document link exists anywhere in the bundle** | grep across `zip-export.ts`, `annotate.ts`, `pipeline.ts` |
| `BatchEntityRegistry.docEntityMap` already holds doc→entity ids, needed for §5.1.2's set arithmetic | `lib/registry.ts:34,99-103` |
| `lib/content-hash.ts` exists and is already tested — reusable for stable ids | `lib/content-hash.ts`, `lib/content-hash.test.ts` |
| **There is no test file for `registry.ts`, `kg-export.ts`, or `zip-export.ts`** | `ls lib/*.test.ts` |
| `worker/pipeline.test.ts` (406 lines) covers `renderAnnotatedMarkdown`, `filterExportableEntities`, `buildEntityFile`, `buildGlossaryIndex` through the pipeline re-exports | `worker/pipeline.test.ts:11-26` |
| Unit runner is `npm run test:unit` (`vitest run`); e2e is `npm run test:e2e` (playwright) — **but both are gated behind a `pretest`/`predev` wasm build that fails on this host; see Task 0.2** | `apps/hacienda-studio/package.json:9,20-23` |
| `model_card.rs` states "Entities truncated at 8 subword tokens" and lists it as a limitation | `hacienda-core/src/compliance/model_card.rs:92,116` |
| The real limit is 8 **words**: `decode_span_scores` indexes `[MAX_COUNT, num_words, MAX_WIDTH, num_labels]` and maps via `words[start].start()..words[end_word-1].end()`, where `words` come from a regex word splitter, not BPE | xberg `candle/decode.rs:45-100`, `v2/splitter.rs:29-35` |

**Assumed — must be confirmed during implementation:**

| Assumption | How to confirm | Task |
|---|---|---|
| Every PII finding overlapping a person/org entity carries a pseudonym token in `redact_template` when the key exists | Assert in a unit test that mask-shaped templates never reach the token path. **Still assumed after Task 0** — the gate probe confirmed person/org findings are *produced*, but supplied synthetic `redact_template` values, so the minting path itself is unverified | 3 |
| No downstream consumer parses `works_for` out of `kg-export/` today | grep the repo + ask before merging; the format is user-facing output | 1 |
| IDF weighting over a real batch produces a useful `## Related documents` list rather than everything-links-to-everything | Measure on a ≥20-file fixture batch before fixing the threshold | 5 |
| Bundles already exported with `works_for` edges exist in users' hands | Unknowable from here — hence the manifest generation marker in Task 1.4 | 1 |

---

## Task 0 — Record the baseline

**Status: COMPLETE — recorded 2026-08-24.** Two corrections to this plan fell out of it (Task 2's
command, and the `npm run test:unit` entry point); both are applied below.

Environment: `feat/pseudonymization-ui-audit-reveal` @ `1ab9c35`, vitest 2.1.9.

### 0.1 Results

- [x] **Unit tests: 224 passed, 0 failed, 22 files, 11.39s.** Clean.
      ⚠️ **but not via `npm run test:unit`** — see 0.2. Measured with `npx vitest run`.
- [x] **`npx tsc --noEmit`: NOT clean at baseline — 9 errors, exit 2.** Task 1–6 diffs must not be
      blamed for these:
      - `tesseract-wasm` has no resolvable declarations under its `package.json` "exports"
        (`lib/pdf-liteparse.test.ts:2`, `lib/pdf-liteparse.ts:12`, `worker/pipeline.ts:217`)
      - implicit `any` params: `lib/pdf-liteparse.ts:68`, `worker/pipeline.ts:272,273`
      - `pages/Settings.tsx:245` — a `string` assigned to the vertical union
      - `pages/Settings.tsx:256,257` — **`sensitivity` does not exist on `AppConfig`**. Verified
        present at `HEAD`, so it is committed-state, not the working tree's WIP. A settings control
        writing a config field nothing declares or reads — the same dead-config class as the
        `redactPiiInOutput` bug fixed in `50a509a`. Out of scope here; flagged separately.
- [x] **e2e: NOT RUNNABLE on this host.** Playwright browsers *are* installed
      (`~/.cache/ms-playwright/chromium-1234`), but `playwright.config.ts:32` starts the suite with
      `npm run dev`, whose `predev` hook is the same wasm build that fails in 0.2. e2e is therefore
      **not a gate for this plan** until that build is fixed. Stated explicitly rather than skipped.
- [x] **`cargo test -p hacienda-core compliance` — matched 0 tests** (`0 passed; … 1 filtered out`).
      **The command in this plan's Task 2 was wrong.** `model_card.rs` contains no `#[test]` at all;
      the compliance tests live in `checklist.rs` and `compliance/mod.rs`. Task 2 corrected.
- [x] **Task 3 gate: CONFIRMED.** Measured with a temporary probe (`lib/__task0-gate.test.ts`,
      created, run, deleted — tree confirmed clean afterwards), in two parts against the **real wasm
      engine**, not a mock:
      - Feeding `person` and `organization` model entities to `scanForPiiWithModelEntities` returns
        both as findings: `[{c:person,s:0,e:11},{c:organization,s:21,e:29}]`.
      - `filterExportableEntities(entities, findings, /* redactPiiInOutput */ true)` → **0 entities
        kept**. The same call with `false` → 2 kept.

      §3.2's analysis holds exactly as written: in `pseudonymize` mode the entity layer is emptied of
      every person and organisation. **Task 3 is unblocked and correctly scoped.**
- [ ] **Reference bundles: NOT produced.** Blocked by 0.2 — this needs a running dev server plus the
      ~614 MB model. The *gate* question they were meant to answer is settled above by direct
      measurement, but the **regression baseline they were also meant to provide does not exist**.
      Tasks 1, 3 and 6 each say "compare against Task 0's reference bundle"; until this is produced,
      those comparisons must be done another way or the task is not verifiable. **Do not treat this
      checkbox as optional.**

### 0.2 Blocker found: the wasm build fails on this host

`npm run test:unit` never reaches vitest. Its `pretest:unit` hook runs
`scripts/build-wasm-if-needed.sh`, which decides a rebuild is needed and then fails:

```
error[E0463]: can't find crate for `candle_nn`   (vendor/candle-transformers-…/colpali.rs:7)
error: could not compile `comrak` … maybe you need to install the missing components with:
       rustup component add rust-src rustc-dev llvm-tools-preview
```

Two separate problems, both worth recording:

1. **The freshness check false-positives.** It compares mtimes, and
   `crates/hacienda-wasm/src/lib.rs` is mtime-newer than `pkg/hacienda_wasm_bg.wasm` while being
   **byte-identical to `HEAD`** (`git status` clean). So a rebuild is triggered by a touched file
   with no content change — the committed `pkg/` is not actually stale.
2. **The rebuild itself is broken here.** `.cargo/config.toml` sets `rustc-wrapper = "sccache"`, and
   "can't find crate" for a dependency that is present, plus `comrak` demanding toolchain
   components, is the signature of a stale or corrupt sccache cache rather than a source defect.
   Not diagnosed further — out of scope for this plan, but it blocks e2e and `npm run test:unit`
   for anyone on this machine.

**Workaround for this plan:** run unit tests as `npx vitest run`. The committed `pkg/` is current in
content, so this tests the same artifact the hook would have produced.

### 0.3 Working tree was dirty when measured

Four files carry uncommitted modifications that are **not** part of this plan's work — in-progress
pseudonym key-id validation:

```
 M apps/hacienda-studio/components/PiiPanel.tsx
 M apps/hacienda-studio/lib/asset-loader.ts
 M apps/hacienda-studio/lib/pseudonym-keys.ts
 M apps/hacienda-studio/pages/Settings.tsx
```

The 224-test and 9-error figures above are measured **against this tree, not against `HEAD`**. The
`Settings.tsx` typecheck errors were individually confirmed to exist at `HEAD` and so are not caused
by this WIP — but the unit-test count was not re-measured on a clean tree. Re-baseline if this work
lands or is reverted.

---

## Task 1 — Stop asserting unobserved relationships (spec §8 step 2)

**Status: COMPLETE — implemented 2026-08-24, verified in worktree `.claude/worktrees/studio-kb-export-task1`.**

**Why first:** this is the only defect that puts *false statements* into a compliance product's output, and the bundle's own README tells readers to trust it over the source text. Missing data (Task 3) is less harmful than wrong data.

**Note on how this landed:** implemented directly in the shared working tree, then interrupted by
a second, independent session (the spawned Settings-field fix, task_2594572f) committing to the
*same* checkout — not an isolated worktree — which at one point swept an uncommitted edit of mine
into one of its own commits, and later checked out a different branch entirely, wiping this task's
uncommitted files from disk. Nothing was lost: an automatic pre-checkout safety commit
(`b4afee4`, a stash-shaped merge of the staged and untracked state) preserved everything, and it
was recovered file-for-file into a proper isolated worktree before continuing. Recorded here
because it is exactly the kind of shared-state hazard `superpowers:executing-plans` should isolate
against — implement multi-task plans like this one in a dedicated worktree from the start, not the
main checkout, when there is any chance another session touches the same repo.

### 1.1 Retype the inferred edges

- [x] In `lib/registry.ts`, replace the four typed emissions in `inferRelationships` (`works_for` ×2, `partner_of` ×2, `contact_email` ×4) with a single `co_occurs_with` edge per unordered entity pair.
      → confirmed: `inferRelationships` rewritten; every emission is `co_occurs_with`
      (`lib/registry.test.ts`'s first test asserts the old three type strings never appear).
- [x] Drop the reciprocal double-emission for organisation pairs (`:169-182` currently emits both directions of a symmetric relation). One edge per unordered pair; record direction-independence in `RegistryRelationship`'s doc comment.
      → confirmed: the new loop iterates `i < j` once per pair and calls `addRelationship` at most
      once per pair; pinned by `should_emit_one_edge_per_unordered_pair`.
- [x] Replace the hardcoded confidences with a proximity score. Minimum viable: same sentence > same paragraph > same document. `RegistryEntity.spans` are not retained on the registry today — either thread the spans through `addEntity` (they are already a parameter, `registry.ts:47`) or compute proximity in `inferRelationships`' caller where spans are still in scope. **Prefer the latter**; widening registry state is a bigger change than this task needs.
      → confirmed, with a deliberate deviation from "prefer the latter": implemented as a private
      `docEntitySpans: Map<docId, Map<entityId, EntitySpan[]>>` side map on the registry instead of
      threading spans through the caller. Reason found during implementation: `inferRelationships`
      is called once per document from `worker/pipeline.ts`, well after `processFile`'s local
      `entities` array (which holds spans) has gone out of scope — the caller does not have spans
      available at the point it calls `inferRelationships`, only `registry` and `docId`. Storing
      them on the registry (populated for free inside the existing `addEntity` call) was the
      smaller change. Not exposed in `toJSON()` — bundle-irrelevant, per-document-only data.
- [x] Keep `context` informative rather than the constant string `"Co-occurs in document"` — record which proximity band produced the score.
      → confirmed: `context` is `"Named in the same sentence"` or `"Named in the same paragraph"`.
- [x] **Added, not in original scope — a real bug found while implementing this task:**
      `addEntity`'s repeat-mention branch (`registry.ts:92-98` pre-task) updated
      `source_documents`/`mention_count` but never `docEntityMap`, so an entity first seen in
      doc-001 and mentioned again in doc-002 was invisible to `inferRelationships(doc-002, …)` —
      co-occurrences involving any previously-seen entity were silently never scored in every
      document after the first. Fixed via a shared `registerInDocument` helper called from both
      branches of `addEntity`. Pinned by `registers a repeat appearance of an entity in a later
      document (Task 1 bug fix)`.
- [x] **Added, not in original scope:** a hard distance cap (`MAX_PROXIMITY_GAP_CHARS = 300`) inside
      `classifyProximity`, independent of the sentence/paragraph check. Without it, "no blank line
      between two mentions" is not actually a proximity signal — a long, unbroken paragraph has no
      length limit, so two entities pages apart with no blank line between them would score
      `same_paragraph`. This was caught empirically: an early version of the "does not explode"
      test (40 persons + 5 orgs, one contiguous paragraph, no blank lines) produced 820 edges — all
      `co_occurs_with`, so not the *old* bug, but evidence the *new* scheme had its own unbounded
      case. The final test (`does not explode on a large document: entities spaced past the
      proximity window get no edge at all`) proves the fix structurally: entities spaced beyond the
      cap, anywhere in a paragraph-break-free document, produce **zero** edges regardless of count —
      not merely "fewer than N", which would only have been a coincidence of the test's shape.

### 1.2 Propagate to the exports

- [x] `lib/kg-export.ts`'s `toCypher` emits `:${r.relationship_type.toUpperCase()}` (`:32`), so the Cypher label follows automatically — **verify** it produces `:CO_OCCURS_WITH` and not something malformed.
      → confirmed by inspection: `"co_occurs_with".toUpperCase()` → `CO_OCCURS_WITH`, valid Cypher
      relationship-type syntax. No code change needed here.
- [x] Check `toRDF`'s predicate construction for the same, and `toNetworkX`'s `type` field.
      → confirmed: `toRDF` interpolates `xberg:${r.relationship_type}` → `xberg:co_occurs_with`
      (valid Turtle predicate, no escaping needed since the value contains no reserved characters);
      `toNetworkX`'s `type: r.relationship_type` is a plain string field, no constraint to satisfy.
      Neither required a code change — both were already generic over the relationship type string.
- [x] Update the bundle README's "Prefer these over re-deriving relationships from the prose" line (`lib/zip-export.ts:135-137`). It is now only true for *co-occurrence*, which is a much weaker claim. State what the edges do and do not mean.
      → confirmed: replaced with a "## What the graph edges mean" section stating the proximity
      bands, that `confidence` is a proximity strength not a relation probability, that
      co-occurrence is explicitly *not* a relationship claim, and that document-level pairs (no
      edge) are already recoverable from `document_entities` rather than repeated as an edge.

### 1.3 Tests (new file)

- [x] Create `lib/registry.test.ts` — there is none today.
      → confirmed: 7 tests (4 required below, 3 added — see 1.1's bug-fix and explosion-fix items,
      plus one confirming `inferRelationships` emits nothing when `text` is omitted).
- [x] `should_not_emit_any_typed_employment_relation`: register a person and an organisation in one document; assert no relationship has type `works_for`, `partner_of`, or `contact_email`.
      → confirmed as `never emits a typed employment, ownership, or contact relation`.
- [x] `should_emit_one_edge_per_unordered_pair`: two organisations in one document → exactly one edge, not two.
      → confirmed as `emits exactly one edge per unordered pair, not one per direction`.
- [x] `should_score_same_sentence_above_same_document`: two entities close together outscore two far apart.
      → confirmed as `scores same-sentence proximity higher than same-paragraph` (retitled: the
      implementation scores same-sentence vs. same-paragraph, not vs. same-document — same-document
      alone, beyond the 300-char cap, scores no edge at all, so "same document" was never a
      confidence tier to compare against).
- [x] `should_not_explode_on_a_large_document`: 40 persons + 5 organisations must not produce ~200 typed claims. Assert the edge count and that every type is `co_occurs_with`.
      → confirmed, retargeted during implementation — see 1.1's explosion-fix note for why an
      arbitrary count threshold (calibrated against the *old* scheme's incidental ~200) was replaced
      with a structural zero-edges-when-spaced-past-the-cap assertion.
      Result: `cd apps/hacienda-studio && npx vitest run lib/registry.test.ts` → 7/7 passed.
      Full suite: `npx vitest run` → 234/234 passed, 23 files (was 224/22 at Task 0 baseline; +10
      for this file). `npx tsc --noEmit` → same 7 pre-existing-post-Task-0 errors, none new or
      removed by this task (see Task 0's baseline note on why the count moved from 9 to 7 already,
      before this task, due to unrelated concurrent commits).

### 1.4 Make the generations distinguishable

- [x] Add a marker to `_manifest.json` recording the relationship semantics version (the spec's §6). A bundle exported before this task and one exported after must be tellable apart without diffing edge lists.
      → confirmed: `_manifest.json` gains `relationshipSemanticsVersion: 2` (1 implicitly being every
      bundle exported before this task).

---

## Task 2 — Correct the model card (spec §8 step 2b)

**Status: COMPLETE — implemented and verified 2026-08-24.**

Two lines, no dependency on any other task. Landed independently, before Task 3.

- [x] `hacienda-core/src/compliance/model_card.rs:92` — "Entities truncated at 8 subword tokens" → 8 **words**. Keep `MAX_WIDTH=8` in the text; only the unit is wrong.
      → confirmed, with the fix widened by one word beyond the literal instruction: the same line
      also said "MAX_WIDTH=8 **token** window limitation" earlier in the sentence, which is the
      identical unit error — "token" there reads as GLiNER2's subword tokenizer, the same
      misidentification the second half of the sentence made explicitly. Left as "token" it would
      still imply the wrong unit even with "8 subword tokens" fixed. Changed both to "word" so the
      sentence is internally consistent: `"Tokenization via DeBERTa tokenizer with MAX_WIDTH=8 word
      window limitation. Entities truncated at 8 words. Negative sampling for false-positive
      reduction."`
- [x] `:116` — same correction in the limitations list.
      → confirmed: `"MAX_WIDTH=8 token limitation may miss…"` → `"MAX_WIDTH=8 word limitation may
      miss…"`.
- [x] Check whether any test asserts on the old string — **answered in Task 0: no.** `grep -rn subword`
      across the workspace (excluding vendor) hits only `model_card.rs:92` itself. Nothing to update.
      → re-verified post-edit on this worktree (Task 0's grep predates two concurrent commits that
      landed on the branch since): `grep -rn "subword\|MAX_WIDTH=8 token" --include=*.rs .` (excl.
      vendor/target) returns nothing. Confirms both old phrasings are fully gone, not just line :92.
- [x] `cargo test -p hacienda-core compliance_` — **corrected from `… compliance`, which matches 0
      tests** (Task 0). `model_card.rs` has no tests of its own; the relevant coverage is
      `compliance/mod.rs`'s `should_name_the_configured_model_in_the_generated_artefacts` and
      `checklist.rs`'s three tests. Run `cargo test -p hacienda-core --lib compliance::` and confirm
      it actually selects them before relying on it as a gate.
      → confirmed and gate proven meaningful: `cargo test -p hacienda-core --lib compliance::` →
      **8 passed, 0 failed** (not the 0-selected silent-pass Task 0 caught with the wrong command).
      Cold build in this fresh worktree, 5m36s, no incremental cache — expected for a first build
      here per the vertical-specialisation plan's own baseline note on this workspace's build times.

**Scope held:** no model-card rewrite. Only the unit was wrong on the two touched lines; everything
else — `pii_categories`, the metrics, the other three limitations — is untouched.

---

## Task 3 — Pseudonym-keyed entity files (spec §8 step 3)

**Status: COMPLETE — implemented and verified 2026-08-24.**

**Gate: PASSED (Task 0.1).** Measured against the real wasm engine: person and organisation model
entities do become PII findings, and `filterExportableEntities(…, true)` retains **0** of them
(vs. 2 with `false`). §3.2 holds; this task is correctly scoped. No re-scope needed.

### 3.1 Exempt pseudonymized entities from the export filter

- [x] `filterExportableEntities` (`worker/pipeline.ts:451-457`) currently keys only on `redactPiiInOutput`. Give it the redaction mode, and when the mode is `pseudonymize` **and** a key was actually derived, retain the entity instead of dropping it.
      → confirmed, with a deliberate deviation from the literal instruction: **not** given a
      `redactionMode` parameter. Instead, the per-finding shape check (3.2 below) decides retention:
      mask (`[EMAIL]`), hash (`#email:1a2b…`), and remove (`""`) templates all fail
      `looksLikePseudonymToken`'s three-part pattern on their own, so the function cannot be told
      the batch is pseudonymized while the findings it actually receives disagree — one fewer
      parameter for a caller to get out of sync. Documented in the function's own doc comment.
- [x] The `pseudonymKeyHex === null` fallback matters: with no passphrase, pseudonymize silently degrades to mask-shaped templates (`worker/pipeline.ts:861-864`). In that case the entity **must still be dropped** — there is no token to key it on, and retaining it would leak the real name into `entities/`. This is the single most important correctness condition in this task.
      → confirmed: this case is exactly what the shape check catches — a degraded pseudonymize
      finding's `redact_template` is mask-shaped, fails `looksLikePseudonymToken`, entity dropped.
      Pinned by `still drops the entity when pseudonymize has no key (mask-shaped fallback
      template)`.

### 3.2 Key the retained entities on their token

- [x] For a retained entity, find the PII finding whose span overlaps it and read its `redact_template` — that is the token. Tokens are minted at `:857`, before filtering at `:905`, so the value is available.
      → confirmed: `overlaps[0].redact_template`. An entity can have multiple overlapping findings
      (repeat mentions within one document); the code requires **all** of them to be real tokens
      before retaining, and takes the first — safe because AES-SIV determinism (below) guarantees
      every mention of the same real name already minted the identical token.
- [x] Entity file name becomes token-derived, not surface-derived: `entities/person-<base32-suffix>.md`. **The filename must not contain the real name** — that is the whole point.
      → confirmed, with a naming deviation: the slug is a `tokenSlug()` FNV-1a hex digest of the
      *whole* token (`worker/pipeline.ts`, colocated with `slugify`), not a literal base32
      substring. A raw base32 suffix would still be reversible with the batch's own key sitting
      right there in the same token string, and is needlessly long for a filename; an 8-hex-char
      hash is short, filesystem-safe, and collision-cheap at realistic batch entity counts. Not a
      security boundary — same non-cryptographic-hash rationale as `lib/redaction-modes.ts`'s
      existing Hash-mode `fingerprint`, documented in `tokenSlug`'s own comment.
- [x] `display_name` / the file's `# ` heading becomes the token, e.g. `[PERSON:session:MFRGG...]`.
      → confirmed: `Entity.name` is overwritten with the token, which flows unchanged into
      `RegistryEntity.canonical_name`/`display_name` (`registry.addEntity` copies `entity.name`
      into both) and therefore into `buildEntityFile`'s `# ${entity.display_name}` heading with no
      further change needed at that layer.
- [x] Registry dedup keys on the token too. AES-SIV is deterministic, so the same person yields the same token across every document in the batch — this is what makes cross-document backlinks work without disclosure. **Assert this property in a test**; it is load-bearing and it is a property of SIV, not something this code enforces.
      → confirmed and asserted against the **real** `mintToken`/`deriveKeyHex` (not a hand-crafted
      fixture, unlike this task's other tests) — see `gives the same real-world entity the same
      token across two documents…` below. Proves the chain end to end: same real name → identical
      token in two independent `mintToken` calls → identical `filterExportableEntities` output →
      `BatchEntityRegistry.addEntity`'s existing `${normalizedName}|${type}|${vertical}` dedup key
      merges them, `source_documents: ["doc-001", "doc-002"]`, with **zero change to
      `registry.ts`'s dedup logic** — it already worked correctly once fed a stable name.
- [x] Verify the same for `GLOSSARY.md`, `entities-registry.json` and `kg-export/`: tokens everywhere, surface forms nowhere.
      → confirmed for `GLOSSARY.md` and entity files directly (next item's test, built via
      `buildGlossaryIndex`/`buildEntityFile` against a real `BatchEntityRegistry`).
      `entities-registry.json` is `registry.toJSON()`, a plain serialisation of the same
      `RegistryEntity` objects already asserted token-only — no separate leak path.
      `kg-export/`'s three formats (`lib/kg-export.ts`) all read from `registry.getEntities()`
      the same way; not independently re-tested here since Task 1 already established (§1.2) that
      all three exporters are generic over entity/relationship field values with no special-casing
      that could reintroduce a surface name.

### 3.3 Tests

- [x] `should_retain_pseudonymized_entities_when_a_key_exists`.
      → confirmed as `retains an entity whose overlapping finding carries a real pseudonym token,
      rekeyed on it`.
- [x] `should_still_drop_entities_when_pseudonymize_has_no_key` — the degradation case from 3.1.
      → confirmed as `still drops the entity when pseudonymize has no key (mask-shaped fallback
      template)`, plus an added test for the multi-span case: `drops the entity if any one of
      several overlapping findings lacks a real token`.
- [x] `should_never_write_a_surface_name_into_a_pseudonymized_bundle`: build a bundle from a fixture containing a distinctive name, then assert that name appears in **no** zip member — filenames included. This is the regression test that matters most.
      → confirmed as `never writes a surface name into a pseudonymized bundle: entity file,
      glossary, and registry all key on the token` — narrower than "no zip member" (does not go
      through `assembleZip`/JSZip; that needs the browser environment Task 0 found unavailable on
      this host), but exercises the real chain up to the point a zip would be built:
      `filterExportableEntities` → `BatchEntityRegistry.addEntity` → `buildGlossaryIndex` →
      `buildEntityFile`, asserting both surface names absent from every output at every stage,
      slugs included (not just prose content).
- [x] `should_give_one_entity_the_same_token_across_documents`.
      → confirmed as `gives the same real-world entity the same token across two documents, so
      cross-document dedup keys on it for free` — the one test in this suite that calls the real
      `mintToken`/`deriveKeyHex`, per 3.2's note above.
- [x] Confirm `mask`/`hash`/`remove` behaviour is unchanged. **Task 0 could not produce the reference
      bundles this was meant to diff against** (Task 0.1, last item). Substitute: assert at the
      `filterExportableEntities` level that all three modes return the identical entity set before and
      after this task's change — a unit-level equivalent that needs no browser. If the reference
      bundles become available, do the byte-level diff as originally written; the unit assertion is
      weaker and does not cover filename or glossary changes.
      → confirmed as `leaves mask, hash, and remove entity sets unaffected by this task's change`,
      parametrised over the three template shapes. The pre-existing four tests in
      `filterExportableEntities (Track A2)` (unchanged, not modified by this task) already covered
      this implicitly by continuing to pass unmodified; this test makes the mask/hash/remove
      equivalence explicit and names it, rather than relying on old tests happening not to break.
      Result: `npx vitest run worker/pipeline.test.ts` → 23/23 (17 pre-existing + 6 new). Full
      suite: `npx vitest run` → **240/240 passed, 23 files** (was 234/23 after Task 1; +6 for this
      task). `npx tsc --noEmit` → same 7 pre-existing-post-Task-0 errors, none new, none removed.

      **Reconciliation note, 2026-08-25:** Task 1's fix (co-occurrence, above) was independently
      re-implemented on the shared base branch as commit `a48b92b`, with a materially better
      identity/alias layer than this plan's original Task 1 — French/English honorific stripping
      and subset-match alias merging for *persons* (`personNameTokens`/`isSubsetMatch`), which this
      plan never built, plus organisation legal-suffix stripping via a suffix-set loop (handles
      cascading suffixes like "Ltd Inc", which this plan's Task 4 regex-based version does not).
      Reconciled by taking `a48b92b`'s `registry.ts`/`registry.test.ts` as the base for Task 1's
      contribution (confirmed byte-for-byte superset of what Task 1 alone changed) and layering
      Task 4's async/SHA-256 stable-id work on top of it, rather than discarding either side's
      work. See Task 4's own reconciliation note below for what that layering involved.

---

## Task 4 — Stable identity (spec §8 step 4)

- [ ] Replace `ent-NNN` (`lib/registry.ts:81`) with a content-derived id — `hash(type|canonical_name|vertical)` via `lib/content-hash.ts`. Under Task 3's pseudonym mode, derive from the token instead, so identity stays stable without being reversible.
- [ ] Do the same for `doc-NNN` (`worker/pipeline.ts:1111`); document ids have the identical instability and feed `document_entities` in the registry JSON.
- [ ] Populate `aliases` (`registry.ts:91`), which nothing writes today: merge on normalised form with legal-suffix stripping (`SAS`, `S.A.S.`, `SARL`, `SA`, `Ltd`, `GmbH`, `Inc`). Longest surface form wins `display_name`; every merged variant lands in `aliases`.
- [ ] Entity filenames follow the merged canonical form, so "Acme", "Acme SAS" and "ACME S.A.S." resolve to one file.
- [ ] Test: re-registering the same corpus in a different document order yields identical ids and identical filenames.
- [ ] Test: alias merging does **not** merge genuinely distinct entities — include a near-miss pair in the fixture.

---

## Task 5 — The structural layer (spec §8 step 5)

### 5.1 Agent instruction files

- [ ] Add `buildAgentInstructions()` to `lib/zip-export.ts` returning **one** string; write it to both `CLAUDE.md` and `AGENTS.md`. Byte-identical, one source — the spec's §4 reasoning is that a bundle explaining the pseudonym contract to one runtime and not the other is worse than one that explains it to neither.
- [ ] Content: the routing table (spec §5.1.1) and the redaction contract. The contract must reflect the **actual** mode of this batch, not a hardcoded paragraph — a mask-mode bundle must not claim tokens are stable identities.
- [ ] Test: both members exist, are byte-identical, and the redaction paragraph varies with mode.

### 5.2 Document→document edges

- [ ] Compute shared-entity similarity from `docEntityMap`, IDF-weighted so an entity present in nearly every document contributes ~nothing.
- [ ] Emit `## Related documents` with the **reason** attached (which entities are shared), not a bare link list.
- [ ] Measure on a ≥20-document batch before fixing the cutoff — the assumption that this yields a useful list rather than a fully-connected graph is untested. Record the measurement in this plan.

### 5.3 Frontmatter, indexes, jsonl

- [ ] Replace the single-line JSON `entities:` blob (`worker/pipeline.ts:482`) with multi-line YAML; add `entity_ids`, `related`, `doc_type` (derived, and labelled as derived — no classifier head exists, spec §2.2).
- [ ] Gate `GLOSSARY.md` to top-N by mention count with links out to `indexes/by-type/<type>.md`. Measure N against a real corpus rather than adopting the spec's placeholder 50 unexamined (spec §9 Q4).
- [ ] Add `_index/entities.jsonl` and `_index/documents.jsonl` — one record per line, so a grep returns a parseable record.
- [ ] Add `indexes/timeline.md` from date entities.
- [ ] **Check the `resolveExportContent` interaction:** `lib/export-resolve.ts:24-38` reconstructs exported markdown by slicing frontmatter with `/^---\n[\s\S]*?\n---/` and the glossary with `lastIndexOf("\n## Entities\n\n")`. Changing frontmatter shape or appending new trailing sections can silently break the K2/I4 edit-override path. Test the override paths explicitly after this change.

---

## Task 6 — Entity dossiers (spec §8 step 6)

**Depends on Task 3** — quoted context must come from post-redaction markdown, or a pseudonymized bundle leaks through its own entity files.

- [ ] Extend `buildEntityFile` (`lib/zip-export.ts:32-55`) with per-mention quoted context (±120 chars), taken from the **redacted** document body.
- [ ] Add ranked co-occurring entities and observed date range.
- [ ] Test: for a pseudonymized bundle, no quoted context contains a surface name — reuse Task 3.3's whole-bundle scan.
- [ ] Watch bundle size: quoted context is the first thing here that scales with mention count, not
      entity count. Task 0's reference batch does not exist — measure the delta on whatever fixture
      batch this task's tests use, and state the fixture's size so the number is interpretable.

---

## Out of Scope

Named explicitly so no task quietly grows into them:

- **Tier 1 semantic labels** (deleting the `:775`/`:811` filter). Gated on the spec's §8 step 1 evaluation harness, which does not exist.
- **Tier 2 head ports** (classifier, relations). Upstream `xberg-gliner` work; re-verified as still unported on upstream `main`.
- **The `xberg` version bump.** Separately recommended against; nothing here depends on it.
- **`## Key facts` blocks.** Tier 1 output, and the spec's §9 Q5 has not settled whether they belong in `documents/*.md` or a sidecar.

---

## Verification Before Merge

- [ ] `cd apps/hacienda-studio && npx vitest run` — at least 224 passing (Task 0's count), 0 failing.
      **Not `npm run test:unit`**, which cannot run here (Task 0.2).
- [ ] `npx tsc --noEmit` — no *new* errors beyond Task 0's 9. The baseline is not zero; compare
      against the enumerated list, not against a clean run.
- [ ] `cargo test -p hacienda-core --lib compliance::` green (Task 2) — and confirm it selects a
      non-zero number of tests, since the obvious filter spelling silently matches none.
- [ ] e2e: **not a gate.** Blocked by Task 0.2's wasm build failure. If that gets fixed, run
      `npm run test:e2e` and record it; do not claim e2e coverage until then.
- [ ] Re-export a reference batch in all four redaction modes and explain every difference.
      **Requires Task 0's outstanding item** — a working dev server and the ~614 MB model. If still
      blocked at merge time, say so explicitly in the PR rather than implying this was checked.
- [ ] **The disclosure test is the gate:** for a pseudonymize-mode bundle, no zip member — content or filename — contains a surface name from the fixture.
- [ ] Open the resulting bundle in Claude Desktop and in a Codex-family agent and confirm both pick up
      their instruction file and can answer a cross-document question without reading every file in
      `documents/`. That is the actual acceptance criterion for the whole plan, and no unit test
      substitutes for it.

      ⚠️ **This criterion is currently unreachable.** It needs a real exported bundle, which needs the
      dev server and model that Task 0.2 blocks. Every task below Task 2 can be *implemented* and
      unit-tested without it, but the plan cannot be **accepted** until the wasm build is fixed. Treat
      unblocking Task 0.2 as a prerequisite for merge, not for starting work.
