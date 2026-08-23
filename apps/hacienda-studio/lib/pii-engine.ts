/**
 * PII detection and redaction, backed by `hacienda-core`'s regex engine compiled to
 * wasm32 (`crates/hacienda-wasm`) — not a TypeScript reimplementation. Replaces
 * `lib/pii-detector.ts` (Track L6 of the 2026-07-30 hacienda program plan): that file
 * had zero importers, so nothing in this app called PII detection at all until this
 * module was wired into `worker/pipeline.ts`.
 *
 * `hacienda_wasm.js`'s `process`/`scan` return `hacienda-core`'s `PipelineResult`
 * serialized via `serde` with no rename — the field names below (`redacted_text`,
 * `format_preserving`, ...) are that struct's actual Rust field names, not a TS
 * convention. Do not camelCase them without also changing the Rust side; the wasm
 * boundary and the CLI/API's own JSON output share that struct.
 *
 * `redactPii` also records every redaction into `AuditHandle` (Track C3) — the
 * IndexedDB-backed blake3 chain from `hacienda-core::audit` (Track L5), compiled to
 * wasm32 the same way the pipeline itself is. Not a second, TypeScript-side audit
 * implementation; see the plan's C3 entry. `scanForPii` never has anything to
 * record — `PiiPipeline::scan` always returns an empty `audit_log`
 * (`hacienda-core/src/pii/pipeline.rs`), because nothing was redacted.
 *
 * The `hacienda-wasm` package (`crates/hacienda-wasm/pkg`, produced by `wasm-pack`)
 * is load**ed lazily** rather than statically imported: it may not exist until
 * `npm run build:wasm` has run, and a static top-level import would make the whole
 * app (including the AI chat, which never touches PII) fail to boot in Vite whenever
 * the build output is missing. See `initPiiEngine` below.
 */
import type { AuditHandle } from "hacienda-wasm";
import { initializeWasmWithCache } from "./wasm-cache";

export interface PiiEntity {
  category: string;
  text: string;
  start: number;
  end: number;
  confidence: number;
  source: string;
  format_preserving: boolean;
  redact_template: string;
}

export interface PiiPipelineResult {
  redacted_text: string;
  entities: PiiEntity[];
}

type WasmModule = typeof import("hacienda-wasm");

let wasmModule: WasmModule | null = null;
let wasmUrl: string | null = null;
let ready: Promise<void> | null = null;

/** Idempotent; safe to call from every worker that needs the engine. */
export function initPiiEngine(): Promise<void> {
  if (!ready) {
    ready = (async () => {
      const mod = await import("hacienda-wasm");
      const { default: url } = await import("hacienda-wasm/hacienda_wasm_bg.wasm?url");
      wasmModule = mod;
      wasmUrl = url;
      // Tier 2.2: served from Cache API/IndexedDB on repeat visits instead of a fresh
      // network fetch — falls back to a plain `fetch(url)` internally on any cache miss.
      await mod.default({ module_or_path: initializeWasmWithCache(url) });
    })();
  }
  return ready;
}

/** Replaces every `wasmModule!` non-null assertion below with one real check — every
 * caller here already awaits `initPiiEngine()` first, so this should never actually
 * throw, but a scattered `!` silently produces "Cannot read properties of null" from
 * deep inside a wasm call if that invariant is ever violated (e.g. a future refactor
 * that calls one of these functions without awaiting init first) instead of a message
 * that says what actually went wrong. */
function getWasmModule(): WasmModule {
  if (!wasmModule) {
    throw new Error(
      "[pii-engine] hacienda-wasm module not initialized — call initPiiEngine() first",
    );
  }
  return wasmModule;
}

/** Detect only — `redacted_text` on the result equals `text`. */
export async function scanForPii(text: string): Promise<PiiPipelineResult> {
  await initPiiEngine();
  return (await getWasmModule().scan(text)) as PiiPipelineResult;
}

/**
 * Loads the Candle GLiNER2 model `process`/`scan` (and therefore `redactPii`/
 * `scanForPii`) use from now on — before this resolves, PII detection is regex-only,
 * exactly as it always was. Takes the same three byte buffers `createNerBackend`
 * (`lib/asset-loader.ts`) already loads Studio's separate entity-glossary `NerModel`
 * from — pass the same fetched bytes here rather than fetching twice. Loading is
 * synchronous on the Rust side (bytes are already in memory), but this is `async`
 * because `initPiiEngine()` must resolve first.
 *
 * Only present when `hacienda-wasm` was built with its `ner-candle-wasm` feature (see
 * `package.json`'s `build:wasm` script) — a build without it doesn't export
 * `loadNerModel` at all, and this call would fail to resolve the import.
 */
export async function loadPiiNerModel(
  weights: Uint8Array,
  tokenizer: Uint8Array,
  encoderConfig: Uint8Array,
): Promise<void> {
  await initPiiEngine();
  getWasmModule().loadNerModel(weights, tokenizer, encoderConfig);
}

/**
 * The wire shape `hacienda-core`'s `PiiCategory` (Rust, `#[serde(rename_all =
 * "snake_case")]`) actually deserializes: a unit variant (`Email`, `PhoneNumber`, ...)
 * is a bare lowercase-snake_case string (`"email"`, `"phone_number"`); the one tuple
 * variant, `Custom(String)`, is externally tagged as `{ custom: "<label>" }` — NOT a
 * bare string, since only unit variants get that shorthand under serde's default
 * (externally tagged) enum representation. `worker/pipeline.ts`'s
 * `nerCategoryToPiiCategory` must produce exactly this shape.
 */
export type PiiCategoryWire = string | { custom: string };

/** The five fields `process_with_model_entities`/`scan_with_model_entities` (and their
 * JS wrappers below) need per entity — shared instead of repeated inline in both. */
export interface ModelEntity {
  category: PiiCategoryWire;
  text: string;
  start: number;
  end: number;
  confidence: number;
}

/**
 * Detect and redact using pre-computed model entities (bypasses NER inference).
 * Model entities should be in the format: { category, text, start, end, confidence }
 * where `category` is the `PiiCategoryWire` shape above (e.g. `"phone_number"` or
 * `{ custom: "Date" }`) — see that type's doc for why it isn't just a string.
 *
 * `process_with_model_entities` only exists on a `hacienda-wasm` build compiled after
 * this function was added on the Rust side — the `pkg/` output actually committed to
 * this repo predates it. Rather than throw for every document until someone runs
 * `npm run build:wasm` against current `crates/hacienda-wasm/src`, fall back to plain
 * `process` (model entities silently unused, same as before this function existed) so
 * processing keeps working against today's committed wasm build.
 *
 * The `typeof` check alone isn't sufficient, either: a `pkg/` rebuild can regenerate
 * `hacienda_wasm.js`'s wasm-bindgen JS glue (which declares this export) without the
 * underlying compiled `hacienda_wasm_bg.wasm` binary actually containing it — e.g. a
 * feature-flag mismatch between the `wasm-pack` invocation that produced the `.js` and
 * the one that produced the `.wasm`. That surfaces as `wasm.process_with_model_entities
 * is not a function` thrown *from inside* the wrapper the `typeof` check just approved
 * (confirmed live: `hacienda_wasm.js`'s `scan_with_model_entities` wrapper existed and
 * passed the check, then threw exactly that on the first real call). Catch and fall
 * back the same way as a missing wrapper, so a half-rebuilt `pkg/` degrades instead of
 * failing every document.
 */
export async function redactPiiWithModelEntities(
  text: string,
  modelEntities: ModelEntity[],
): Promise<PiiPipelineResult> {
  await initPiiEngine();
  const mod = wasmModule as unknown as Record<string, unknown>;
  let result: PiiPipelineResult;
  if (typeof mod.process_with_model_entities === "function") {
    try {
      result = (await (mod.process_with_model_entities as (...args: unknown[]) => Promise<unknown>)(
        text,
        modelEntities,
      )) as PiiPipelineResult;
    } catch (err) {
      console.warn(
        "[pii-engine] process_with_model_entities exists but isn't callable (stale/mismatched wasm build?) — falling back to process():",
        err,
      );
      result = (await getWasmModule().process(text)) as PiiPipelineResult;
    }
  } else {
    result = (await getWasmModule().process(text)) as PiiPipelineResult;
  }
  // Same audit-chain invariant `redactPii` documents above — this path was missing it
  // entirely, so every model-entity redaction (the entity-glossary reuse path, which is
  // the common case now per Tier 1.1) silently never reached the audit trail.
  const handle = await getAuditHandle();
  await handle.recordResult(result);
  return result;
}

/**
 * Detect without rewriting text, using pre-computed model entities (bypasses NER inference).
 * Same committed-wasm-lags-source, wrapper-exists-but-binary-doesn't fallback as
 * `redactPiiWithModelEntities` above.
 */
export async function scanForPiiWithModelEntities(
  text: string,
  modelEntities: ModelEntity[],
): Promise<PiiPipelineResult> {
  await initPiiEngine();
  const mod = wasmModule as unknown as Record<string, unknown>;
  if (typeof mod.scan_with_model_entities === "function") {
    try {
      return (await (mod.scan_with_model_entities as (...args: unknown[]) => Promise<unknown>)(
        text,
        modelEntities,
      )) as PiiPipelineResult;
    } catch (err) {
      console.warn(
        "[pii-engine] scan_with_model_entities exists but isn't callable (stale/mismatched wasm build?) — falling back to scan():",
        err,
      );
    }
  }
  return (await getWasmModule().scan(text)) as PiiPipelineResult;
}

// One IndexedDB database per browser profile — Studio has no concept of multiple
// concurrent writers (one worker, processed sequentially) or of a varying redaction
// config, so `AUDIT_NODE_ID`/`AUDIT_CONFIG_HASH` are stable literals rather than
// something computed per session. `AUDIT_CONFIG_HASH` mirrors
// `AuditConfig::default()`'s own literal ("default") on the native side.
const AUDIT_DB_NAME = "hacienda-studio-audit";
const AUDIT_NODE_ID = "hacienda-studio";
const AUDIT_CONFIG_HASH = "default";

let auditHandle: Promise<AuditHandle> | null = null;

/** Opens (or resumes, across a reload) the audit chain. Idempotent, like `initPiiEngine`. */
function getAuditHandle(): Promise<AuditHandle> {
  if (!auditHandle) {
    auditHandle = getWasmModule().AuditHandle.open(
      AUDIT_DB_NAME,
      AUDIT_NODE_ID,
      AUDIT_CONFIG_HASH,
    );
  }
  return auditHandle;
}

/**
 * Detect and mask (the pipeline's default redaction mode), then record the batch of
 * redactions this call produced into the audit chain.
 *
 * One `recordResult` call per document, matching the one-`append`-per-document
 * invariant `HaciendaFacade::record_audit` enforces natively — this function is the
 * whole of one document's processing, so that invariant holds for free here.
 *
 * A failed audit write fails this call rather than being swallowed: an audit trail
 * that silently drops entries on error is worse than a visible failure for a feature
 * whose entire purpose is being trustworthy.
 */
export async function redactPii(text: string): Promise<PiiPipelineResult> {
  await initPiiEngine();
  const result = (await getWasmModule().process(text)) as PiiPipelineResult;
  const handle = await getAuditHandle();
  await handle.recordResult(result);
  return result;
}

/**
 * The chain's current head, for the redesign's Audit tab. Reflects every redaction
 * recorded so far in this browser profile (`redactPii` appends per-document, not
 * per-tab), not one specific document's own entry — `AuditHandle` doesn't expose a
 * per-document lookup today, only the running tip and whole-chain `verify()`. Good
 * enough to prove "nothing in the chain has been tampered with since it was written";
 * a document-scoped view would need a new wasm-side method (see the redesign plan's
 * Audit tab note).
 */
export async function getAuditChainTip(): Promise<string> {
  await initPiiEngine();
  const handle = await getAuditHandle();
  return handle.tip();
}

/** Recomputes every chain hash from genesis; resolves on success, throws on the first
 * mismatch (never returns a boolean a caller could accidentally ignore). */
export async function verifyAuditChain(): Promise<void> {
  await initPiiEngine();
  const handle = await getAuditHandle();
  await handle.verify();
}

/**
 * Records that `revealedText` was shown to the user in plaintext — the wasm-side
 * `RedactionAction::Reveal` entry (`hacienda-core/src/audit/entry.rs`), appended by
 * `AuditHandle::recordReveal`. Only `revealedText`'s blake3 digest is ever hashed into
 * the chain; the plaintext itself is never sent anywhere but back to the caller's own
 * `setRevealed` state (`components/PiiPanel.tsx`).
 *
 * `source` matches `PiiEntity.source` ("regex" or "model"). Returns the chain's new tip.
 */
export async function recordPiiReveal(
  revealedText: string,
  category: string,
  source: string,
): Promise<string> {
  await initPiiEngine();
  const handle = await getAuditHandle();
  return handle.recordReveal(revealedText, category, source);
}

/** One row of `hacienda-core`'s `AuditEntry` (`hacienda-core/src/audit/entry.rs`),
 * serialized as-is — same no-rename convention `PiiEntity` above documents. */
export interface AuditEntryRow {
  id: string;
  timestamp: string;
  category: string;
  action: "mask" | "hash" | "pseudonymize" | "remove" | "reveal" | { custom: string };
  span_hash: string;
  span_length: number;
  confidence: number | null;
  source: string;
  pipeline_version: string;
  config_hash: string;
  principal: string | null;
  vertical: string | null;
  chain_hash: string;
}

/** Every entry recorded so far, oldest first — backs `DocumentDetail.tsx`'s Audit tab. */
export async function listAuditEntries(): Promise<AuditEntryRow[]> {
  await initPiiEngine();
  const handle = await getAuditHandle();
  return (await handle.listEntries()) as AuditEntryRow[];
}
