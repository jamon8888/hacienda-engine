/* tslint:disable */
/* eslint-disable */

/**
 * One open [`IndexedDbAuditStore`] connection, exposed to JS.
 */
export class AuditHandle {
    private constructor();
    free(): void;
    [Symbol.dispose](): void;
    /**
     * Open (or resume, across a reload — Track L5's check) the IndexedDB database
     * named `db_name`, scoped to `node_id` and `config_hash`.
     *
     * `wasm-bindgen` cannot expose an `async` constructor, so this is a static
     * factory: `await AuditHandle.open(...)` from JS.
     */
    static open(db_name: string, node_id: string, config_hash: string): Promise<AuditHandle>;
    /**
     * Record one processed document's redactions as a single batch — one `append`
     * per document, the same invariant `HaciendaFacade::record_audit` enforces, so
     * a chain built here is structurally identical to one built server-side.
     *
     * `result` is the `JsValue` a caller already got back from [`super::process`].
     * Entries with an empty `audit_log` (nothing was redacted) append nothing and
     * return the chain's current tip unchanged.
     *
     * # Errors
     *
     * Rejects if `result` doesn't deserialize as a `PipelineResult`, or if the
     * store rejects the batch (e.g. the chain was closed).
     */
    recordResult(result: any): Promise<string>;
    /**
     * The chain's current head — a client that records this alongside a result can
     * later prove which chain state produced it.
     */
    tip(): Promise<string>;
    /**
     * Recompute every chain hash from genesis. Throws (rejects) on the first
     * mismatch rather than returning a boolean, so a caller can't accidentally
     * ignore tampering by discarding a return value.
     */
    verify(): Promise<void>;
}

export function isNerModelLoaded(): boolean;

/**
 * Load the model `process`/`scan` will use from now on, replacing any previously
 * loaded one. Bytes are already fully in memory (Studio fetches and IndexedDB-caches
 * them once, shared with its own separate entity-glossary NER pass) — this is
 * synchronous, no I/O happens here.
 *
 * # Errors
 *
 * Throws if the model bytes cannot be loaded — this function is synchronous (per its
 * own doc above), so a JS caller sees a thrown exception, not a rejected Promise.
 */
export function loadNerModel(weights: Uint8Array, tokenizer: Uint8Array, encoder_config: Uint8Array): void;

/**
 * Regex + NER (when a model is loaded, via [`ner_model::load_ner_model`]) detection and
 * redaction over `text`, using the default pipeline config. Returns the serialized
 * `PipelineResult`. `with_detector` (rather than `new`) is used unconditionally: it
 * ignores `PipelineConfig::model.enabled` and just uses whatever `Option<NerDetector>`
 * it's handed, so this is identical to today's regex-only behaviour whenever no model
 * is loaded (or the `ner-candle-wasm` feature is off) — no `#[cfg]` needed here.
 */
export function process(text: string): Promise<any>;

/**
 * Detect and redact using pre-computed model entities (bypasses NER inference).
 * Takes pre-computed model entities as a JS array of objects with fields:
 * category, text, start, end, confidence.
 */
export function process_with_model_entities(text: string, model_entities: any): Promise<any>;

/**
 * Redact `text` under `mode` ("mask", "hash", "pseudonymize", or "remove") with no
 * detected spans — exercises `RedactionEngine::redact`'s construction path, including
 * the AES-SIV `Pseudonymiser`, without depending on `PiiPipeline` detection.
 */
export function redact_empty(text: string, mode: string): any;

/**
 * Detect without rewriting `text`. Returns the serialized `PipelineResult` with
 * `redacted_text` equal to the input.
 */
export function scan(text: string): Promise<any>;

/**
 * Detect without rewriting `text`, using pre-computed model entities (bypasses NER inference).
 */
export function scan_with_model_entities(text: string, model_entities: any): Promise<any>;

export type InitInput = RequestInfo | URL | Response | BufferSource | WebAssembly.Module;

export interface InitOutput {
    readonly memory: WebAssembly.Memory;
    readonly __wbg_audithandle_free: (a: number, b: number) => void;
    readonly audithandle_open: (a: number, b: number, c: number, d: number, e: number, f: number) => any;
    readonly audithandle_recordResult: (a: number, b: any) => any;
    readonly audithandle_tip: (a: number) => any;
    readonly audithandle_verify: (a: number) => any;
    readonly isNerModelLoaded: () => number;
    readonly loadNerModel: (a: number, b: number, c: number, d: number, e: number, f: number) => [number, number];
    readonly process: (a: number, b: number) => any;
    readonly process_with_model_entities: (a: number, b: number, c: any) => any;
    readonly redact_empty: (a: number, b: number, c: number, d: number) => [number, number, number];
    readonly scan: (a: number, b: number) => any;
    readonly scan_with_model_entities: (a: number, b: number, c: any) => any;
    readonly wasm_bindgen_bf7b0d491ce864c2___convert__closures_____invoke___wasm_bindgen_bf7b0d491ce864c2___JsValue__core_8c5caaf0847c1b83___result__Result_____wasm_bindgen_bf7b0d491ce864c2___JsError___true_: (a: number, b: number, c: any) => [number, number];
    readonly wasm_bindgen_bf7b0d491ce864c2___convert__closures_____invoke___web_sys_c2086d39a3c4ab29___features__gen_IdbVersionChangeEvent__IdbVersionChangeEvent__core_8c5caaf0847c1b83___result__Result_____wasm_bindgen_bf7b0d491ce864c2___JsValue___true_: (a: number, b: number, c: any) => [number, number];
    readonly wasm_bindgen_bf7b0d491ce864c2___convert__closures_____invoke___js_sys_f311ed201db48e3e___Function_fn_wasm_bindgen_bf7b0d491ce864c2___JsValue_____wasm_bindgen_bf7b0d491ce864c2___sys__Undefined___js_sys_f311ed201db48e3e___Function_fn_wasm_bindgen_bf7b0d491ce864c2___JsValue_____wasm_bindgen_bf7b0d491ce864c2___sys__Undefined_______true_: (a: number, b: number, c: any, d: any) => void;
    readonly wasm_bindgen_bf7b0d491ce864c2___convert__closures_____invoke___web_sys_c2086d39a3c4ab29___features__gen_Event__Event______true_: (a: number, b: number, c: any) => void;
    readonly wasm_bindgen_bf7b0d491ce864c2___convert__closures_____invoke_______true_: (a: number, b: number) => void;
    readonly __wbindgen_malloc: (a: number, b: number) => number;
    readonly __wbindgen_realloc: (a: number, b: number, c: number, d: number) => number;
    readonly __wbindgen_exn_store: (a: number) => void;
    readonly __externref_table_alloc: () => number;
    readonly __wbindgen_externrefs: WebAssembly.Table;
    readonly __wbindgen_destroy_closure: (a: number, b: number) => void;
    readonly __externref_table_dealloc: (a: number) => void;
    readonly __wbindgen_start: () => void;
}

export type SyncInitInput = BufferSource | WebAssembly.Module;

/**
 * Instantiates the given `module`, which can either be bytes or
 * a precompiled `WebAssembly.Module`.
 *
 * @param {{ module: SyncInitInput }} module - Passing `SyncInitInput` directly is deprecated.
 *
 * @returns {InitOutput}
 */
export function initSync(module: { module: SyncInitInput } | SyncInitInput): InitOutput;

/**
 * If `module_or_path` is {RequestInfo} or {URL}, makes a request and
 * for everything else, calls `WebAssembly.instantiate` directly.
 *
 * @param {{ module_or_path: InitInput | Promise<InitInput> }} module_or_path - Passing `InitInput` directly is deprecated.
 *
 * @returns {Promise<InitOutput>}
 */
export default function __wbg_init (module_or_path?: { module_or_path: InitInput | Promise<InitInput> } | InitInput | Promise<InitInput>): Promise<InitOutput>;
