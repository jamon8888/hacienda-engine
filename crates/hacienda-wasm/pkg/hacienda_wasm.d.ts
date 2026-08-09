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

/**
 * Compresses multiple entries into a 7z archive in WebAssembly environment.
 *
 * This function creates a compressed archive from multiple file entries,
 * designed specifically for WASM targets.
 *
 * # Arguments
 * * `entries` - Vector of JavaScript strings representing file names/paths
 * * `datas` - Vector of Uint8Arrays containing the file data corresponding to entries
 */
export function compress(entries: string[], datas: Uint8Array[]): Uint8Array;

/**
 * Decompresses a 7z archive in WebAssembly environment.
 *
 * This function is specifically designed for WASM targets and uses JavaScript interop
 * to handle the decompression process with a callback function.
 *
 * # Arguments
 * * `src` - Uint8Array containing the compressed archive data
 * * `pwd` - Password string for encrypted archives (use empty string for unencrypted)
 * * `f` - JavaScript callback function to handle extracted entries
 */
export function decompress(src: Uint8Array, pwd: string, f: Function): void;

/**
 * Regex + NER (when a model is loaded) detection and redaction over `text`, using the
 * default pipeline config. Returns the serialized `PipelineResult`.
 */
export function process(text: string): Promise<any>;

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

export type InitInput = RequestInfo | URL | Response | BufferSource | WebAssembly.Module;

export interface InitOutput {
    readonly memory: WebAssembly.Memory;
    readonly __wbg_audithandle_free: (a: number, b: number) => void;
    readonly audithandle_open: (a: number, b: number, c: number, d: number, e: number, f: number) => any;
    readonly audithandle_recordResult: (a: number, b: any) => any;
    readonly audithandle_tip: (a: number) => any;
    readonly audithandle_verify: (a: number) => any;
    readonly process: (a: number, b: number) => any;
    readonly redact_empty: (a: number, b: number, c: number, d: number) => [number, number, number];
    readonly scan: (a: number, b: number) => any;
    readonly compress: (a: number, b: number, c: number, d: number) => [number, number, number];
    readonly decompress: (a: any, b: number, c: number, d: any) => [number, number];
    readonly wasm_bindgen_dfe9ebb593cb7bad___convert__closures_____invoke___wasm_bindgen_dfe9ebb593cb7bad___JsValue__core_9b3796e30d99ddb7___result__Result_____wasm_bindgen_dfe9ebb593cb7bad___JsError___true_: (a: number, b: number, c: any) => [number, number];
    readonly wasm_bindgen_dfe9ebb593cb7bad___convert__closures_____invoke___web_sys_861182ea0de7500c___features__gen_IdbVersionChangeEvent__IdbVersionChangeEvent__core_9b3796e30d99ddb7___result__Result_____wasm_bindgen_dfe9ebb593cb7bad___JsValue___true_: (a: number, b: number, c: any) => [number, number];
    readonly wasm_bindgen_dfe9ebb593cb7bad___convert__closures_____invoke___js_sys_c8702d64e55b495e___Function_fn_wasm_bindgen_dfe9ebb593cb7bad___JsValue_____wasm_bindgen_dfe9ebb593cb7bad___sys__Undefined___js_sys_c8702d64e55b495e___Function_fn_wasm_bindgen_dfe9ebb593cb7bad___JsValue_____wasm_bindgen_dfe9ebb593cb7bad___sys__Undefined_______true_: (a: number, b: number, c: any, d: any) => void;
    readonly wasm_bindgen_dfe9ebb593cb7bad___convert__closures_____invoke___web_sys_861182ea0de7500c___features__gen_Event__Event______true_: (a: number, b: number, c: any) => void;
    readonly wasm_bindgen_dfe9ebb593cb7bad___convert__closures_____invoke_______true_: (a: number, b: number) => void;
    readonly __wbindgen_malloc_command_export: (a: number, b: number) => number;
    readonly __wbindgen_realloc_command_export: (a: number, b: number, c: number, d: number) => number;
    readonly __wbindgen_exn_store_command_export: (a: number) => void;
    readonly __externref_table_alloc_command_export: () => number;
    readonly __wbindgen_externrefs: WebAssembly.Table;
    readonly __wbindgen_destroy_closure_command_export: (a: number, b: number) => void;
    readonly __externref_table_dealloc_command_export: (a: number) => void;
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
