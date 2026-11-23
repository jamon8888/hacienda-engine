/**
 * SHA-256 digests for every asset loaded by the browser NER backend.
 *
 * Single source of truth: `model-assets/registry.json`, shared with the native
 * hosts through `hacienda-model-assets`. A model selected through
 * VITE_GLINER_MODEL_ID must be registered there before Studio will use it, and
 * a cache hit is never trusted merely because bytes exist in IndexedDB.
 */
import registry from "../../../model-assets/registry.json";

export interface ModelDigests {
  model: string;
  tokenizer: string;
  encoder: string;
}

function digestsFor(assets: ReadonlyArray<{ role: string; sha256: string }>): ModelDigests {
  const pick = (role: string): string => {
    const asset = assets.find((entry) => entry.role === role);
    if (!asset) throw new Error(`Model registry entry is missing the '${role}' asset`);
    return asset.sha256;
  };
  return { model: pick("model"), tokenizer: pick("tokenizer"), encoder: pick("encoder") };
}

export const MODEL_DIGESTS: Record<string, ModelDigests> = Object.fromEntries(
  registry.models.map((entry) => [entry.id, digestsFor(entry.assets)]),
);

export type ModelId = keyof typeof MODEL_DIGESTS;
