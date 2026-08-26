/**
 * Mode choisi dans le panneau Conversion (Format original / Format Markdown).
 * `original` conserve la mise en page d'origine (non normalisée),
 * `markdown` produit du Markdown normalisé via le pipeline WASM.
 */

export type ConversionMode = "original" | "markdown";

/**
 * Format attendu par le WASM (`hacienda-core` `WasmOutputFormat`).
 * `plain` = texte brut / format original, `markdown` = Markdown.
 */
export type WasmOutputFormat = "plain" | "markdown";

/**
 * Mappe le choix UI vers la valeur wire WASM.
 */
export function conversionToWasmFormat(mode: ConversionMode): WasmOutputFormat {
  if (mode === "markdown") return "markdown";
  return "plain";
}

/**
 * Inverse mapping — utile pour initialiser l'UI depuis une préférence persistée.
 */
export function wasmFormatToConversionMode(fmt: WasmOutputFormat): ConversionMode {
  if (fmt === "markdown") return "markdown";
  return "original";
}
