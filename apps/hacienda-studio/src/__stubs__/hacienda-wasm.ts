/**
 * Stub for `hacienda-wasm` used only when the wasm32 build has not been produced
 * (`crates/hacienda-wasm/pkg` is absent — i.e. `npm run build:wasm`/wasm-pack has
 * not run). It makes the dev server resolve the import so the app (including the
 * AI chat, which never touches PII) boots. Calling any PII function throws a clear
 * message instead of a cryptic module-resolution failure.
 *
 * Wired conditionally in `vite.config.ts`: when `crates/hacienda-wasm/pkg` exists,
 * the real wasm-bindgen package is used instead and this stub is never loaded.
 */

function notBuilt(fn: string): never {
  throw new Error(
    `${fn} requires hacienda-wasm, which is not built. Run \`npm run build:wasm\` in ` +
      "apps/hacienda-studio (installs/uses wasm-pack against crates/hacienda-wasm) " +
      "to compile the PII engine to wasm32.",
  );
}

async function init(_opts: { module_or_path: unknown }): Promise<unknown> {
  notBuilt("hacienda-wasm init");
  return undefined;
}

export default init;

export async function scan(_text: string): Promise<unknown> {
  notBuilt("scanForPii");
}

export async function process(_text: string): Promise<unknown> {
  notBuilt("redactPii");
}

export function loadNerModel(
  _weights: Uint8Array,
  _tokenizer: Uint8Array,
  _encoderConfig: Uint8Array,
): void {
  notBuilt("loadPiiNerModel");
}

export const AuditHandle = {
  open() {
    notBuilt("AuditHandle");
    return undefined as never;
  },
};
