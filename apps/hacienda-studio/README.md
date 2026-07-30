# hacienda-studio

Browser-only workspace for local, zero-egress document redaction. Runs entirely client-side
(extraction, NER, PII detection/redaction, entity linking, knowledge-graph export) in a Web
Worker — no document ever leaves the browser.

## Development

```bash
npm install
npm run dev      # builds crates/hacienda-wasm's pkg/ first (predev), then starts Vite
npm run test:unit
npm run test:e2e
npm run build
```

## Relationship to the CLI/API — target = one engine, not two

Studio and `hacienda-cli`/`hacienda-api` are two different trust models — Studio runs in the
browser with no server round-trip, the CLI/API run server-side — but they are **not meant to
stay two implementations forever**. The target has always been one engine, `hacienda-core`
compiled to `wasm32-unknown-unknown`, shared by both. Track L
(`docs/superpowers/plans/2026-07-30-hacienda-program-plan.md`) is most of the way there:

- **PII detection and redaction: unified.** Studio calls the same `hacienda-core` regex
  engine the CLI/API do, compiled to wasm32 (`crates/hacienda-wasm`, wired in via
  `lib/pii-engine.ts`). One shared fixture corpus (`fixtures/pii-corpus.json`) is asserted
  from both `cargo test` and `vitest` against the real compiled wasm build. There is no
  separate TypeScript PII detector anymore.
- **Audit chain: unified in kind, not in durability.** Studio's audit trail is the same
  blake3-chained `AuditStore` the CLI/API use, backed by IndexedDB instead of a file
  (`hacienda-core/src/audit/store_idb.rs`). It survives a page reload, but not a cleared
  browser profile — a file-backed chain survives the process. Whether/how the browser's
  chain gets exported into a longer-lived vault for legally-defensible output is still an
  open question.
- **Still two implementations, not yet one:**
  - **Reversible pseudonymization** (`[CATEGORY:key_id:BASE32]`, AES-SIV,
    `redaction/pseudonym.rs`) exists only in Rust today. Studio doesn't expose it yet. If
    it's built in TypeScript against WebCrypto, it must match the Rust token format exactly
    — a document pseudonymized in one must be revealable by the other. Do not invent a
    second format.
  - **NER differs, and not in the way it looks.** The pipeline's live entity extraction
    (`worker/pipeline.ts` → `XbergEngine.ner`) is wired to `lib/ner-bridge.ts`'s
    regex/`compromise.js` bridge, not to `@xberg-io/xberg-wasm`'s neural `NerModel`
    (GLiNER2) — that model is downloaded into IndexedDB and has a loader
    (`asset-loader.ts`'s `createNerBackend()`), but nothing calls it. `hacienda-core`'s own
    neural NER (`ner-candle`) is separately compiled out of every default build, browser or
    server. Track L didn't touch either; there's no current plan to wire up the unused
    GLiNER2 loader.

**Do not "helpfully" port a Rust detector into TypeScript, and do not wire Studio to
`/v1/pii/scan`/`/v1/pii/redact`.** Those routes are real and serve a different deployment
with a different trust model — they are not Studio's backend. If Studio is missing PII
coverage or redaction behavior the CLI/API already have, the fix is compiling more of
`hacienda-core` to wasm32, not reimplementing it here.
