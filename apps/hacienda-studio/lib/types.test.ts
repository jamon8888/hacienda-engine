import { describe, expect, it } from "vitest";
import { WasmEntityCategory } from "@xberg-io/xberg-wasm";
import { DEFAULT_CONFIG, type NerCategory } from "./types";

/**
 * `NerCategory` is a hand-maintained mirror of the engine's `WasmEntityCategory`
 * enum, and a name outside that vocabulary is not a soft failure: the engine
 * refuses to deserialize the whole NER result and the app reports "Unknown
 * error" against the document. Shipping `phone_number` instead of `phone` broke
 * every file containing a phone-shaped number — including French IBANs, which
 * the phone regex matches. Pin the mirror to the enum it mirrors.
 */
const ENGINE_CATEGORIES = new Set(
  Object.keys(WasmEntityCategory)
    .filter((key) => Number.isNaN(Number(key)))
    .map((key) => key.toLowerCase()),
);

describe("NER category vocabulary", () => {
  it("mirrors every non-custom variant of the engine's enum", () => {
    const mirrored: NerCategory[] = [
      "person",
      "organization",
      "location",
      "date",
      "time",
      "money",
      "percent",
      "email",
      "phone",
      "url",
    ];
    expect(new Set([...mirrored, "custom"])).toEqual(ENGINE_CATEGORIES);
  });

  it("defaults to categories the engine accepts", () => {
    for (const category of DEFAULT_CONFIG.nerCategories) {
      expect(ENGINE_CATEGORIES).toContain(category);
    }
  });
});
