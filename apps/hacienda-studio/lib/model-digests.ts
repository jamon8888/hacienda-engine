// Digest pinning: first 8 hex chars of SHA-256, computed over Uint8Array content
// # ponytail: real digests not yet published; placeholders trigger fail-closed path
export const MODEL_DIGESTS = {
  "jamon8888/gliner2-guardrails-pii-f16": {
    model: "53c73fff",
    tokenizer: "ab12abcd",
    encoder: "cd34ef56",
  },
  "fastino/gliner2-privacy-filter-PII-multi": {
    model: "00000000",
    tokenizer: "11111111",
    encoder: "22222222",
  },
} as const;

export type ModelId = keyof typeof MODEL_DIGESTS;
