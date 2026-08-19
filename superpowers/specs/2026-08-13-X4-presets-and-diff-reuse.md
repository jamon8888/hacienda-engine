# X4 — Presets and diff engine: reuse vs. reinvent

**Date:** 2026-08-13
**Status:** Proposed
**Program:** `2026-08-13-hacienda-xberg-capability-parity-program.md` §8
**Depends on:** nothing shipped
**Blocks:** nothing

---

## 1. Problem

hacienda already reinvented two things xberg ships as reusable modules, confirmed by reading
both sides directly rather than assuming from naming:

**Presets.** xberg's `presets` feature (`crates/xberg/src/presets/`) ships a format, a
registry (`Registry::load_embedded`/`extend_from_dir`), and a **resolver**
(`xberg::presets::resolve`) that merges a base preset with overrides by a configurable
`MergeMode`. hacienda's `/v1/presets` is a separate Postgres-backed CRUD store
(`hacienda_core::store::postgres::presets::PresetStore`, `hacienda-api/src/handlers/presets.rs`)
with none of that machinery — confirmed: the two share no code, and hacienda's presets are a
plain named-config row, not a resolved/merged pipeline configuration.

**Diff.** xberg's `diff` feature (`crates/xberg/src/diff/`, pure Rust via the `similar`
crate) structurally diffs two `ExtractedDocument`s — table cells, embedded content, hunks
with context lines. hacienda's `/v1/documents/{id}/diff` (spec E2) instead diffs on
pseudonymised tokens: the platform-parity program's own decision D3 requires this, since
diffing raw text would violate I1 (no unredacted text crosses a persistence/response
boundary). That constraint is real and correct — what's unverified is whether xberg's *diff
engine* could be the mechanism **underneath** E2's pseudonym-token contract, rather than E2
reimplementing text diffing from scratch.

## 2. Objectives / Non-objectives

**Objectives**

- A written decision, per feature, on whether to adopt xberg's module as the underlying
  engine — not a rewrite done speculatively before the decision is made.

**Non-objectives**

| Deferred | Reason |
| --- | --- |
| Implementing either "adopt" outcome | Follow-up work once the decision is made, not part of this spec's deliverable |
| Reopening D5 (presets are commodity, minimal scope) | This spec asks a narrower question — does the *resolver specifically* close a real gap — not whether presets deserve more product investment generally |

## 3. Presets: does the resolver buy anything the CRUD store doesn't?

The platform-parity program's D5 already rendered a verdict on named/versioned presets
generally: "commodity, no bid stops there," scope kept minimal. This spec's question is
narrower and specific to the resolver: **merge-mode composition — a base preset plus
per-call overrides, resolved once — is a capability, not just a naming convenience.** A
customer with twenty near-identical extraction configurations (one per document type, each
overriding two or three fields from a shared base) either maintains twenty full rows in
hacienda's CRUD store, or gets that composition for free from xberg's resolver.

**Decision to make:** survey whether any current or requested hacienda preset use case
actually needs composition (vs. twenty independent full configs being an acceptable cost).
If yes, adopting `xberg::presets::resolve` as the resolution engine underneath
`PresetStore` (storing base + override rows, resolving on read) is worth scoping as a
follow-up. If no — D5's original verdict holds, confirmed rather than merely inherited.

## 4. Diff: can xberg's engine run on pseudonymised documents?

**The question is narrow and mechanical:** `xberg::diff::compare` takes two
`ExtractedDocument` values and has no side effects — no I/O, per its own doc comment. If E2
feeds it two already-pseudonymised `ExtractedDocument`s (produced after the P1/P7 guard has
already run on each version), the diff engine never sees unredacted content, and I1 is not
reopened. What xberg's engine adds over E2's own text diff: table-cell-aware diffing,
embedded-content diffing, structured hunks with context lines — real capability, not
cosmetic, for documents where the interesting changes are in a table (a contract's pricing
schedule, a financial statement) rather than in prose.

**Two things to verify before treating "adopt" as safe, not assume:**

- `compare`'s output (`ExtractionDiff`, with `TableDiff`, `CellChange`, etc.) must not
  reconstruct or infer values across the diff in a way that defeats pseudonymisation's
  determinism guarantee (D3 in the platform-parity program: same value → same token, so a
  diff over two pseudonymised versions is exact *as long as nothing in the diff engine tries
  to be clever about near-matches at the character level inside a token*, which would treat
  two different pseudonym tokens as "similar" text and produce a misleading partial-match
  hunk). Read `crates/xberg/src/diff/mod.rs`'s hunk-generation logic specifically for this
  before adopting.
- E2's existing "refuse to diff across a key rotation" rule (D3, condition 1) must still be
  enforceable with xberg's engine underneath — confirm the refusal happens before
  `compare` is called, not that `compare` is expected to know about pseudonym key rotation
  (it can't; that's E2's concern, not xberg's).

## 5. Exit criteria

- A written decision for presets: adopt the resolver (with a scoped follow-up spec) or
  confirm D5's verdict, with the reason stated either way — this spec's actual deliverable.
- A written decision for diff: adopt `xberg::diff` as E2's underlying engine (with the two
  verifications in §4 confirmed) or confirm E2's current from-scratch implementation stays,
  with the reason stated either way.
- If "adopt" for diff: E2's existing tests (its pseudonym-token-diff contract, unchanged from
  a caller's perspective) continue to pass — the engine changes underneath, the contract
  doesn't.

## 6. Sequencing

Independent of P6/P7/X1/X2/X3. Lowest priority in the program — a leverage/correctness
question about work already done, not a missing capability blocking anything else.
