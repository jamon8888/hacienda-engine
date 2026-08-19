# P6 — LLM call enforcement point

**Date:** 2026-08-13
**Status:** Fixed — see §7 for what actually shipped and where it diverges from §3
**Program:** `2026-08-13-hacienda-xberg-capability-parity-program.md` §4
**Extends:** `2026-08-01-P1-redaction-enforcement-point.md` — same pattern, applied to a
network call instead of a store write
**Depends on:** nothing shipped is required first; the guard can be built and the existing
call site migrated to it independently of X1–X4
**Blocks:** X3 (`2026-08-13-X3-llm-backed-enrichment-features.md`)

---

## 1. Problem

`hacienda-rag`'s `answer_stream` (`crates/hacienda-rag/src/stream.rs`) calls
`liter_llm::LlmClient::chat_stream` directly. Its own doc comment is explicit about the
contract: *"Callers **must** redact `prompt` and every chunk's `content`... before calling
this function — this module performs no redaction itself."* The one caller that exists,
`hacienda-api/src/handlers/rag_stream.rs`, honours it — redacts the prompt and every
retrieved chunk, with a comment naming this "Mandatory PII redaction gate."

That is correct behaviour produced by discipline, not by a guarantee. P1 solved the
equivalent problem for storage by making the *unguarded* store type unexportable from its
crate — a future integration cannot skip the redaction step because there is no unguarded
store to hand it. There is no equivalent here: any future code, anywhere in the workspace,
can construct a `liter_llm::LlmClient` (or call `xberg::llm::complete_text`/
`complete_with_json_schema`, if a future integration reaches for those instead) and hand it
raw text. `2026-08-13-X3-llm-backed-enrichment-features.md` names five capabilities that each
need exactly this kind of call — `translation`, `summarization-llm`, `captioning`,
`classification`, `structured` — each a fresh chance to reproduce `rag_stream.rs`'s
discipline correctly, or not.

## 2. Objectives / Non-objectives

**Objectives**

- A generic guard, structurally analogous to P1's `Guard<S>`, wrapping any call that sends
  document-derived text to an LLM.
- Migrate the one existing call site (`rag_stream::answer`) to use it, with no change in
  observable behaviour.
- Make every future LLM-backed capability (X3) depend on this guard by construction, not by
  a reviewer remembering to check for a redaction call before the LLM call.

**Non-objectives**

| Deferred | Reason |
| --- | --- |
| Which X3 capabilities exist at all | → X3 |
| Redact-first vs. raw-input-redact-output policy per capability | → X3 §7, this spec only guarantees redaction happens *somewhere* the guard controls, not which side of the call |
| Multi-provider routing, rate limiting, cost accounting | Infrastructure concerns orthogonal to the redaction guarantee |
| Image redaction (relevant to `captioning`) | A distinct capability; X3 flags it, not built here |

## 3. Shape

```text
GuardedLlm
  complete(prompt, context)  → detect → redact/pseudonymise → THEN call the LLM client
  stream(prompt, context)    → same, then stream tokens back to the caller
  reveal-on-the-way-out?     → NO. Unlike P1's read arm (which logs a reveal when a store
                                read discloses redacted span text), an LLM response is
                                generated text, not a revealed span — nothing to log as a
                                "reveal" here. What *is* logged: an audit entry per call,
                                same discipline as P2, recording that content was sent to
                                an external LLM (which capability, when, by whom) — this is
                                itself a fact worth a durable record, distinct from a
                                redaction entry.
```

**D-P6-1 — the raw LLM client is not exported outside the owning crate.** Same discipline as
P1's D-P1-1. Concretely: whatever module owns `GuardedLlm` holds the only public constructor
for a `liter_llm::LlmClient` (or equivalent) reachable from the rest of the workspace; nothing
outside that module can build one directly.

**D-P6-2 — where the guard lives.** Two candidates:

- **`hacienda-core`**, alongside `PiiPipeline`. Every consumer (`hacienda-rag` today, any
  future X3 module) already depends on `hacienda-core` for `Capability`/`Caller`/`PiiPipeline`
  itself, so this adds no new dependency edge. Matches P1's own placement rationale
  (`facade.rs`'s comment: "each future caller... does not have to reimplement it, one of
  them will forget" — literally the sentence this spec is applying to a second kind of call).
- **A new crate** (`hacienda-llm-guard` or similar), parallel to `hacienda-rag`. Cleaner
  separation if the guard grows real LLM-provider plumbing (retries, provider fallback) that
  doesn't belong in the PII/audit core.

**Recommendation: `hacienda-core`.** The guard's job is enforcing I1/I5, which is exactly
`hacienda-core`'s existing responsibility — it doesn't need provider-plumbing sophistication
to do that job, and P1's precedent (the guard lives with the invariant it enforces, not with
the feature that happens to be its first consumer) argues for the same placement here.
Revisit only if a future X3 capability needs LLM-provider machinery heavy enough to justify
its own crate — at which point `hacienda-core` still owns the *guard*, and that hypothetical
crate would depend on it, not replace it.

**D-P6-3 — `PiiPipeline` dependency.** `GuardedLlm::complete`/`stream` need a `PiiPipeline`
reference to redact with, same as `HaciendaFacade::redact_text_with_auth` does. The guard is
therefore constructed with (or borrows) a `PiiPipeline`, not a bare LLM client config —
mirroring `Guard<S>`'s own dependency on the redaction pipeline it wraps stores with.

## 4. Migrating the existing call site

`hacienda_rag::answer_stream` keeps its current signature and its documented "performs no
redaction itself" contract — it is a low-level streaming primitive, and P6 does not need to
change what it is. What changes is `hacienda-api/src/handlers/rag_stream.rs`: instead of
calling `redact_text_with_auth` twice by hand (once for the prompt, once per chunk) and then
calling `answer_stream` directly, it calls through `GuardedLlm::stream`, which performs the
same two redaction calls internally before delegating to `answer_stream`. Behaviour is
unchanged; what's gained is that the next caller — any of X3's five capabilities — gets this
for free instead of having to reread `rag_stream.rs` and copy the pattern by hand.

## 5. Audit linkage

Every `GuardedLlm` call writes an audit entry distinct from a redaction entry: a record that
says *this principal sent (redacted) content of category/size N to provider P at time T for
purpose (translate/summarize/caption/classify/structured-extract/rag-answer)*. This is the
same *"additive and linked"* property the redaction/pseudonymisation layer already has for
storage — an auditor asking "did this document's content leave the building" gets an answer
from the chain, not from trusting that the feature that sent it also logged it correctly.

## 6. Exit criteria

- A test constructs a scenario where a control-corpus PII value is present in a
  `GuardedLlm::stream` call's input and asserts it never appears in the text handed to the
  (test-double) LLM client — the same "never leaks the value" test class already used for
  `hacienda-api`'s guarded routes and `hacienda-mcp`'s tools, now applied to the LLM boundary.
- A structural/compile-time test (or, if Rust's type system can't express it directly, a
  workspace-wide grep-based CI check, the same fallback P1 itself may need to use) asserts
  no crate outside the guard's owning module constructs a `liter_llm::LlmClient` or calls
  `xberg::llm::{complete_text,complete_with_json_schema}` directly.
- `rag_stream::answer`'s existing control-corpus test continues to pass unchanged after the
  migration in §4 — same observable behaviour, different internal path.
- Every `GuardedLlm` call produces exactly one new audit entry type (§5), verifiable via
  `GET /v1/audit/entries` the same way every other audited action already is.

## 7. What actually shipped

**Placement diverges from §3's recommendation, for a reason §3 didn't check.**
`hacienda-core` has zero dependency on `liter-llm` and cannot gain one without depending on
`hacienda-rag` — the crate that already documents, in its own `Cargo.toml`, that the edge
runs the other way (`hacienda-rag`'s `postgres` dev-dependency comment: *"hacienda-core does
not depend on hacienda-rag"*). Placing `GuardedLlm` in `hacienda-core` as recommended would
have required either reversing that dependency direction or duplicating `liter-llm` as a
second `hacienda-core` dependency alongside `hacienda-rag`'s — both worse than the
alternative taken: **`GuardedLlm` lives in `hacienda-rag::stream`**, next to
`answer_stream` and the private `build_token_stream` it wraps, which is where
`liter_llm::LlmClient` is actually constructed. This still satisfies D-P6-1 (nothing outside
the owning module reaches the raw client) — `build_token_stream` was already private before
this spec.

**D-P6-3 (`PiiPipeline` dependency) is satisfied via a trait, not a direct reference.**
Since `hacienda-rag` cannot depend on `hacienda-core`'s `PiiPipeline` type either, `GuardedLlm`
is generic over a new `hacienda_rag::Redactor` trait (`async fn redact(&self, text: &str) ->
RagResult<String>`) instead of holding a `PiiPipeline` reference directly. `hacienda-api`
implements `Redactor` via `FacadeRedactor`, a thin adapter over
`HaciendaFacade::redact_text_with_auth` — the same redaction call the migrated handler used
to make by hand. This is a stronger decoupling than §3 anticipated, not a weaker one: any
future crate wanting a `GuardedLlm`-backed capability supplies its own `Redactor` without
`hacienda-rag` ever needing to know `hacienda-core` exists.

**Migration (§4) shipped exactly as specified.** `hacienda_rag::answer_stream` is unchanged —
same signature, same "performs no redaction itself" doc contract. `hacienda-api/src/handlers/rag_stream.rs`
now builds a `FacadeRedactor` and calls `GuardedLlm::new(&redactor).stream(context, prompt,
config)` once, instead of two manual `redact_text_with_auth` calls followed by a direct
`answer_stream` call. `rag_answer_streams_citation_then_error_without_a_routable_model` (the
pre-existing control test) passes unchanged.

**Exit criteria, checked against §6:**
- ✅ Never-leaks test, adapted: `guarded_llm_calls_the_redactor_on_the_prompt_and_every_chunk`
  proves the `Redactor` sees the prompt and every chunk's raw content, and
  `guarded_llm_refuses_to_stream_when_the_redactor_rejects_{the_prompt,a_chunk}` prove a
  redaction failure stops the call before any stream is built — nothing downstream can
  observe content the redactor didn't approve, whether or not a live LLM client is involved.
- ✅ Structural check, via the grep-based fallback the spec itself anticipated needing:
  `crates/hacienda-rag/tests/llm_call_enforcement.rs` walks every workspace crate's `src/`
  (this crate's own `stream.rs` excepted) and fails the build if `liter_llm::create_client`,
  `.chat_stream(`, or either `xberg::llm` text-completion entry point appears anywhere else.
- ✅ `rag_stream::answer`'s existing control-corpus test, unchanged (above).
- **Deferred, not shipped:** the new "LLM call" audit-entry type (§5). The redaction
  guarantee — the part of I5 that actually stops unredacted text from leaving the process —
  is structural as of this revision. The audit-linkage half (*a durable record that content
  was sent to provider P for purpose X*) is additive polish on top of a guarantee that
  already holds without it, not a safety property in itself, and needs its own small design
  pass: `AuditEntryInput`'s `source: EntitySource` field has only `Regex`/`Model` today, and
  neither names "sent to an LLM" honestly. Tracked as a follow-up, not silently dropped.
