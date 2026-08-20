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
      await mod.default({ module_or_path: fetch(url) });
    })();
  }
  return ready;
}

/** Detect only — `redacted_text` on the result equals `text`. */
export async function scanForPii(text: string): Promise<PiiPipelineResult> {
  await initPiiEngine();
  return (await wasmModule!.scan(text)) as PiiPipelineResult;
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
  wasmModule!.loadNerModel(weights, tokenizer, encoderConfig);
}

/**
 * Detect and redact using pre-computed model entities (bypasses NER inference).
 * Model entities should be in the format: { category, text, start, end, confidence }
 * where category is a PII category string (e.g., "Person", "Email", "PhoneNumber", etc.)
 */
export async function redactPiiWithModelEntities(
  text: string,
  modelEntities: Array<{
    category: string;
    text: string;
    start: number;
    end: number;
    confidence: number;
  }>,
): Promise<PiiPipelineResult> {
  await initPiiEngine();
  return (await wasmModule!.process_with_model_entities(text, modelEntities)) as PiiPipelineResult;
}

/**
 * Detect without rewriting text, using pre-computed model entities (bypasses NER inference).
 */
export async function scanForPiiWithModelEntities(
  text: string,
  modelEntities: Array<{
    category: string;
    text: string;
    start: number;
    end: number;
    confidence: number;
  }>,
): Promise<PiiPipelineResult> {
  await initPiiEngine();
  return (await wasmModule!.scan_with_model_entities(text, modelEntities)) as PiiPipelineResult;
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
    auditHandle = wasmModule!.AuditHandle.open(
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
  const result = (await wasmModule!.process(text)) as PiiPipelineResult;
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
