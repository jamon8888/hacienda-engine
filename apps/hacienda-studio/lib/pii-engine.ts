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
 */
import initHaciendaWasm, {
  process as wasmProcess,
  scan as wasmScan,
} from "hacienda-wasm";
import haciendaWasmUrl from "hacienda-wasm/hacienda_wasm_bg.wasm?url";

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

let ready: Promise<void> | null = null;

/** Idempotent; safe to call from every worker that needs the engine. */
export function initPiiEngine(): Promise<void> {
  if (!ready) {
    ready = initHaciendaWasm({ module_or_path: fetch(haciendaWasmUrl) }).then(
      () => undefined,
    );
  }
  return ready;
}

/** Detect only — `redacted_text` on the result equals `text`. */
export async function scanForPii(text: string): Promise<PiiPipelineResult> {
  await initPiiEngine();
  return (await wasmScan(text)) as PiiPipelineResult;
}

/** Detect and mask (the pipeline's default redaction mode). */
export async function redactPii(text: string): Promise<PiiPipelineResult> {
  await initPiiEngine();
  return (await wasmProcess(text)) as PiiPipelineResult;
}
