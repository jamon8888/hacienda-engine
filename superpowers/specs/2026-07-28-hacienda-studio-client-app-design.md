# Design Spec: hacienda-studio — Client-Facing Redaction & Compliance App

**Date:** 2026-07-28
**Status:** Draft — pending gate verification (§11)
**Audience:** EU regulated professionals, first vertical: avocats / droit des affaires

---

## 1. Executive Summary

**hacienda-studio** is a browser application that lets a law firm drop documents,
audio and video in, review and correct the detected personal data, and export a
pseudonymised Markdown bundle that is safe to hand to Claude Desktop — together
with evidence that the processing was lawful.

Its defining property is that **document content never leaves the browser**.
Extraction, transcription, NER and PII detection all run locally in WebAssembly.
For an *avocat*, this is worth more than GDPR compliance alone: *secret
professionnel* (loi n° 71-1130, art. 66-5) is the stricter constraint, and it is
what makes uploading client files to a US cloud service a professional risk
rather than merely a regulatory one. Local processing removes that risk, and the
proof pack documents exactly what was — and was not — transmitted.

The product is therefore not "a redaction tool". It is **a defensible chain of
custody that ends with the client safely using an LLM**.

---

## 2. Confirmed Decisions

| # | Decision | Value |
|---|----------|-------|
| 1 | First vertical | Avocats — **droit des affaires**; further verticals to follow |
| 2 | Network | **Allowed**, with the content boundary in §4 |
| 3 | Pseudonymisation | **Reversible** — keyfile + local rehydration |
| 4 | UI language | **Bilingual FR/EN** |
| 5 | UI framework | **React 19 + Extend UI** (migration from Svelte 5, §6) |

### 2.1 The network boundary (explicit)

"Network allowed" is scoped to three uses, and **document content is not one of
them**:

| Permitted | Forbidden |
|-----------|-----------|
| Model + WASM asset download | Document bytes |
| eIDAS qualified timestamping of **hashes only** | Extracted text, transcripts |
| Licence validation | Entity values, pseudonym mappings |

This boundary is the product's core claim. Every dependency and feature is
assessed against it (§11).

---

## 3. Current State

The pipeline was repaired on `feat/studio-integration` (three commits) and now
runs end-to-end for the first time. Prior to that, processing any file failed.

**Working:** 97-format extraction via xberg WASM in a Web Worker; entity
extraction; cross-document entity registry with dedup; knowledge-graph export
(Cypher / NetworkX / RDF); Markdown output with YAML frontmatter and entity
links; JSZip bundle download; IndexedDB asset caching with onboarding.

**Stubbed or absent:**

- `lib/pii-detector.ts` — 9 regexes, US-centric, no model use (§8)
- GLiNER2-Guardrails-PII is downloaded and cached but **never called**
- `compromise.js` supplies person/org/location NER and is **English-only**
- `AppConfig.enablePiiDetection` / `redactPiiInOutput` are wired to nothing
- No review UI of any kind
- `hacienda-core`'s DPIA / model card / DORA / audit chain are unused by the app
- No audio/video (plan exists: `docs/superpowers/plans/2026-07-28-audio-…md`)

**Known defects not yet fixed:**

- `asset-loader.ts:11` loads the model from `/api/huggingface/…`, a **dev-proxy-only**
  path that will return `index.html` in production
- `dictionary.ts:12,20` stamps `sectors[0]` on every entity (all M&A → `technology`)
- `vitest`, `jsdom`, `@testing-library` undeclared → 4 unit tests never run
- `turbo.json` `outputs` wrong → build caching broken
- Page title still reads `xberg-studio`

---

## 4. Trust Model

Zero egress creates a proof problem: with no server, there is no server-side log,
and an auditor cannot verify what code ran. Three mechanisms answer this.

**1. Hash-chained local audit log.** Every detection, dismissal and manual
redaction is appended to a blake3 chain (`hacienda-core/src/audit/chain.rs`,
already implemented). The chain is local and tamper-evident.

**2. eIDAS qualified timestamp on the root hash.** Only the chain's root hash is
transmitted to an RFC 3161 timestamp authority. This yields legally recognised
proof that a given set of decisions existed at a given time, while transmitting
zero content. Degrades gracefully offline (unstamped but still chained).

**3. Reproducible builds + published artefact hashes.** The WASM and JS bundle
hashes are published and verifiable via SRI, with a "verify this build" page.
Without this, "it ran locally in my browser" is unfalsifiable.

> Positioning discipline: we supply **evidence**, not certification. No UI string
> may assert "GDPR compliant". All artefacts state scope and limits.

---

## 5. Architecture

```
┌──────────────────────── BROWSER (content boundary) ────────────────────────┐
│                                                                             │
│  React 19 + Extend UI                                                       │
│  ┌───────────────┬────────────────────┬──────────────────────────────────┐  │
│  │ file-upload   │  Review surface    │  Export / Rehydrate              │  │
│  │ file-system   │  layout-blocks     │  bundle builder, keyfile, proof  │  │
│  │ file-thumbnail│  bounding-box-cit. │                                  │  │
│  └───────────────┴────────────────────┴──────────────────────────────────┘  │
│         │                   │                          │                    │
│  ┌──────┴───────────────────┴──────────────────────────┴────────────────┐   │
│  │ Web Worker — framework-agnostic TypeScript (unchanged by §6)         │   │
│  │  xberg WASM extraction ─ Whisper transcription ─ GLiNER2 PII ─       │   │
│  │  vertical taxonomy ─ entity registry ─ pseudonym allocator ─         │   │
│  │  blake3 audit chain                                                  │   │
│  └──────────────────────────────────────────────────────────────────────┘   │
│                                                                             │
└─────────────────────────────────────────────────────────────────────────────┘
        │ hashes only                    │ assets in
        ▼                                ▼
   eIDAS TSA                     model / WASM CDN
```

---

## 6. UI Framework: React 19 + Extend UI

**Decision:** migrate the UI shell from Svelte 5 to React 19 and adopt
[extend-hq/ui](https://github.com/extend-hq/ui) (MIT, 55 components, source-copied
via the shadcn registry, no Next.js coupling — verified).

**Rationale.** The migration cost is bounded and the gain is large:

| | Lines |
|---|---|
| Framework-coupled (`App.svelte`, 3 lib components, `main.ts`) | 655 |
| Framework-agnostic TypeScript (worker, registry, verticals, transcription, …) | 1,592 |

71% of the codebase is portable as-is; the WASM/worker architecture is untouched.
Against 655 lines of rewrite, three Extend components alone supply ~13,700 lines
of production document tooling.

**Why these components specifically.** `layout-blocks` renders OCR/layout output
as selectable blocks carrying confidence and Markdown, overlaid on the rendered
page. `bounding-box-citations` links a Markdown span to its exact region on the
source PDF. For an avocat reviewing a *garantie d'actif et de passif*, clicking a
detected entity and seeing where it sits in the signed contract is a materially
stronger review than a plain text buffer.

**This supersedes the earlier CodeMirror proposal** for the primary review
surface. CodeMirror remains optional for free-text Markdown editing, but it has
no knowledge of source-document geometry and cannot provide the above.

Also adopted: `file-upload`, `file-system`, `file-thumbnail`, `document-splits`,
and the `docx` / `xlsx` / `pptx` / `csv` viewers.

**Costs:** Tailwind adoption (currently plain CSS); a materially larger dependency
surface to attest to; egress verification required per §11.

---

## 7. Vertical: Droit des Affaires

`m&a.yaml` is a genuine asset — `earnout`, `escrow`, `closing_condition`,
`material_adverse_change`, `governing_law` is a corporate lawyer's vocabulary.
But it encodes **Anglo-American** M&A, and translation is the wrong move:

> *Garantie d'actif et de passif* is not `representation_and_warranty` +
> `indemnification`. It is a structurally distinct instrument. Mapping it onto the
> English pair produces wrong entity links in precisely the documents that matter.

**Decision:** add `droit-des-affaires` as its own taxonomy beside `m&a`, keeping
`shared` as the base, with **jurisdiction as a first-class field**.

Entity types to add: *cession de parts sociales / d'actions, pacte d'associés,
protocole d'accord, lettre d'intention, condition suspensive, clause de
non-concurrence, garantie d'actif et de passif, séquestre, audit d'acquisition*.
French legal-entity identifiers (**SIREN, SIRET, RCS, Kbis, TVA
intracommunautaire**, forms *SAS/SARL/SA/SCI*, *commissaire aux comptes*, *greffe
du tribunal de commerce*) are dual-purpose: taxonomy entries **and** PII patterns.

Because further verticals are coming (droit social, fiscal, IP, contentieux), the
taxonomy format gets its extension points now — while there is exactly one
consumer. This requires replacing the hand-rolled `parseYAML`, which cannot
express nested maps and so cannot carry per-entity aliases.

---

## 8. PII Detection Rework

The current detector fails on French documents in ways that are not tunable:

| Defect | Consequence |
|---|---|
| `phone` is US `\d{3}-\d{3}-\d{4}` | misses **every** French number |
| `ssn` is US SSN | French **NIR** undetected — a special-category identifier |
| `bic` `[A-Z]{6}[A-Z0-9]{2}` | matches ordinary uppercase words |
| `passport` `[A-Z]{1,2}\d{6,9}` | matches invoice and case references |
| IBAN unvalidated | no mod-97 checksum |
| no overlap resolution | IBAN/BIC/card double-match one span |
| `compromise.js` English-only | person/org NER blind on French text |

**Decision:** two-layer detection.

1. **GLiNER2-Guardrails-PII** (multilingual, already downloaded and cached, and
   currently unused) replaces `compromise.js` for person / organisation / location.
2. **Validated EU patterns** replace the regex set: NIR with key check, SIREN/SIRET
   with Luhn, IBAN with mod-97, E.164 phones, TVA intracommunautaire — each with a
   validator, not just a shape, plus deterministic overlap resolution.

**False negatives are the liability story.** We never claim exhaustive detection.
Human review is a mandatory gate before export (§9), measured recall is published
in the model card, and known limits are documented — which the AI Act's
transparency obligations require regardless.

---

## 9. Redaction Model

**Redaction is a layer, never a text mutation.** The current
`redactPII()` splices `[EMAIL]` into the string: irreversible, and it collapses
distinct people into one token, destroying analytical value *and* recoverability.

Instead, the buffer stays immutable and `(span, entity, state)` is held alongside
it — `Entity.spans` already exists in `types.ts`, so the foundation is present.

Three states, each an audit-chain event:

| State | Meaning | Evidence value |
|---|---|---|
| `auto-redacted` | model detected, kept | baseline |
| `dismissed` | human marked false positive | **AI Act human oversight** |
| `manual` | human selected and redacted | catches model false negatives |

Export is gated on review: the operator cannot produce a bundle without
acknowledging the residual re-identification assessment. A control that cannot be
skipped is a control you can point at in an audit.

---

## 10. Pseudonymisation, Export & Rehydration

**Stable reversible pseudonyms.** `[PERSON_1]` rather than `[REDACTED]`, allocated
by the existing `BatchEntityRegistry` so the same person carries the same token
across every document in the batch. The bundle stays fully analysable by an LLM.

**The keyfile never enters the bundle.** The pseudonym→value mapping is written to
a separate encrypted keyfile the firm retains.

**Rehydration closes the loop.** The client sends the bundle to Claude Desktop,
receives an analysis referring to `[PERSON_1]`, pastes it back into
hacienda-studio, and the local keyfile restores real names. This converts a lossy
export into a defensible workflow, and is the feature most likely to differentiate
the product.

**Bundle layout** — shaped for how Claude Desktop reads folders (an Obsidian-style
vault; a `README.md` explaining the pseudonym convention measurably improves
analysis quality):

```
documents/*.md      pseudonymised, entity-linked, YAML frontmatter
entities/*.md       one note per entity, with backlinks
README.md           index + pseudonym convention
_manifest.json      per-file hashes
_compliance/        proof pack (§4): Art. 30 register, Art. 35 DPIA,
                    AI Act model card, DORA ICT entry, audit chain,
                    timestamp token, residual-risk assessment
```

The DORA entry is unusually strong here: there is no ICT third party.

---

## 11. Gates Before Implementation

1. **Egress audit of Extend UI and its dependencies** (embedpdf, tanstack-virtual,
   hugeicons, glide-data-grid, base-ui, dnd-kit). Confirm no runtime fetch of
   fonts, PDF workers, or telemetry. *One background request to a US CDN while a
   client contract is open would undermine the entire product claim.* **Hard gate.**
2. **Production model URL strategy** — `/api/huggingface/…` is dev-proxy-only and
   will break in production (§3).
3. **eIDAS TSA selection** — qualified provider, EU-established.
4. **Legal review of all compliance wording** by the client's counsel. Nothing in
   this spec is a legal opinion.

---

## 12. Phasing

| Phase | Content | Depends on |
|---|---|---|
| 0 | Egress audit; production model URL; consolidate branches | gates 1–2 |
| 1 | React migration + Extend UI shell; port 655 lines; bilingual FR/EN scaffolding | 0 |
| 2 | PII rework — GLiNER2 wiring + validated EU patterns | 0 |
| 3 | `droit-des-affaires` taxonomy; real YAML parser; jurisdiction field | 2 |
| 4 | Review surface — layout-blocks + bounding-box-citations, three-state spans | 1, 2 |
| 5 | Pseudonym allocator, keyfile, rehydration | 4 |
| 6 | Proof pack + audit chain + eIDAS timestamping | 5, gate 3 |
| 7 | Audio/video via `@remotion/whisper-web` (existing plan) | 2 |

Phase 7 note: transcripts only — raw audio is discarded immediately, as voice is
biometric-adjacent and should not persist in IndexedDB. Whisper segments carry
timecodes, so redaction spans map back to audio time; "bleeped audio export" is a
strong differentiator for recorded interviews but is out of scope for v1.
`validateFile`'s 50 MB cap conflicts with this phase and must be revisited.

---

## 13. Open Questions

1. Deployment shape — static site, self-hosted on firm premises, or Tauri desktop?
   The first-run asset cost (48 MB WASM + up to 466 MB Whisper + GLiNER2) argues
   for a desktop build for heavy users.
2. Licensing/enforcement model, given offline operation is a selling point.
3. Retention: does the firm need bundles and keyfiles to survive a browser profile
   reset, and if so where do they live?
