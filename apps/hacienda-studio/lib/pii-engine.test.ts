/**
 * Track C2/L6's shared fixture corpus, asserted against the real wasm32 build — not
 * mocked. `hacienda-core/tests/pii_corpus.rs` asserts the same `fixtures/pii-corpus.json`
 * from `cargo test`; this is "one corpus, two runners, identical output" (Track L6's
 * check) because both runners exercise the exact same regex engine, one natively and
 * one compiled to wasm32.
 *
 * Loads the `.wasm` bytes directly from disk (`node:fs`) rather than through
 * `pii-engine.ts`'s `fetch(...?url)` path: that path is how the browser worker loads the
 * module, but vitest runs in Node (`vitest.config.ts`'s `environment: "node"`), where
 * there is no dev server to answer the `?url` request. `hacienda_wasm`'s init function
 * accepts raw bytes directly, so this is a legitimate second entry point, not a stand-in
 * that skips the real module.
 */
import { createRequire } from "node:module";
import { readFileSync } from "node:fs";
import { describe, expect, it, beforeAll } from "vitest";
import initHaciendaWasm, { scan } from "hacienda-wasm";
import corpus from "../../../fixtures/pii-corpus.json";

// `import.meta.resolve` isn't implemented by Vitest's SSR module transform;
// `createRequire` resolves through plain Node module resolution instead.
const resolve = createRequire(import.meta.url).resolve;

beforeAll(async () => {
  const wasmPath = resolve("hacienda-wasm/hacienda_wasm_bg.wasm");
  await initHaciendaWasm({ module_or_path: readFileSync(wasmPath) });
});

interface PiiEntity {
  category: string;
}

interface PipelineResult {
  entities: PiiEntity[];
}

describe("PII corpus (Track C2/L6)", () => {
  for (const testCase of corpus.cases) {
    it(`detects exactly ${JSON.stringify(testCase.categories)} for '${testCase.id}'`, async () => {
      const result = (await scan(testCase.text)) as PipelineResult;
      const actual = new Set(result.entities.map((e) => e.category));
      const expected = new Set(testCase.categories);
      expect(actual).toEqual(expected);
    });
  }
});
