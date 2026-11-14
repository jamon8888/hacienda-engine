/**
 * SHA-256 digests for every asset loaded by the browser NER backend.
 *
 * These are deliberately full digests, not short display identifiers. The model is a
 * security boundary: a cache hit must not be trusted merely because bytes exist in
 * IndexedDB, and a model selected through VITE_GLINER_MODEL_ID must be registered here
 * before Studio will use it.
 */
export const MODEL_DIGESTS = {
  "jamon8888/gliner2-guardrails-pii-f16": {
    model: "7dd22d082c67e9a358c55870fb4ab466faada59e75559c15ac0d0ae328439efe",
    tokenizer: "f6df10ec83bea993035b2dd7c39345a3d4fcf23421c2adb6cb4ffc1e6d1bc4b5",
    encoder: "f27dd63cc43a248d2566f0b6ad7a115db353676ce0561dcbca45bac766464c1a",
  },
  "fastino/gliner2-privacy-filter-PII-multi": {
    model: "0280f6f39f6012da50b6640bad438d9b7e763a1b0102094115d1b710c4dd79b6",
    tokenizer: "f6df10ec83bea993035b2dd7c39345a3d4fcf23421c2adb6cb4ffc1e6d1bc4b5",
    encoder: "f27dd63cc43a248d2566f0b6ad7a115db353676ce0561dcbca45bac766464c1a",
  },
} as const;

export type ModelId = keyof typeof MODEL_DIGESTS;
