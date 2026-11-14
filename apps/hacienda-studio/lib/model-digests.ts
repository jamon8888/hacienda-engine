export const MODEL_DIGESTS = {
  "jamon8888/gliner2-guardrails-pii-f16": {
    model: "53c73fff",
    tokenizer: "ab12abcd",
    encoder: "f27dd63c",
  },
  "fastino/gliner2-privacy-filter-PII-multi": {
    model: "00000000",
    tokenizer: "11111111",
    encoder: "f27dd63c",
  },
} as const;

export type ModelId = keyof typeof MODEL_DIGESTS;
