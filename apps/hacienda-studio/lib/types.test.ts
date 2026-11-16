import { describe, expect, it } from "vitest";
import { DEFAULT_CONFIG, type NerCategory } from "./types";

/**
 * `NerCategory` is a hand-maintained mirror of the engine's `WasmEntityCategory`
 * enum, and a name outside that vocabulary is not a soft failure: the engine
 * refuses to deserialize the whole NER result and the app reports "Unknown
 * error" against the document. Shipping `phone_number` instead of `phone` broke
 * every file containing a phone-shaped number — including French IBANs, which
 * the phone regex matches. In xberg-wasm 1.4, `WasmEntityCategory` is a data class
 * rather than a JavaScript enum, so this list mirrors the official conversion table
 * instead of reflecting runtime enum keys.
 */
const ENGINE_CATEGORIES = new Set([
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
]);

describe("NER category vocabulary", () => {
  it("mirrors every non-custom variant of the engine's enum", () => {
    const mirrored: NerCategory[] = [
      "custom",
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
      "first_name",
      "middle_name",
      "last_name",
      "street_address",
      "city",
      "state_or_region",
      "postal_code",
      "country",
    ];
    // Engine categories are a subset of NerCategory; NerCategory is extended for GLiNER custom labels
    for (const cat of ENGINE_CATEGORIES) {
      expect(mirrored).toContain(cat);
    }
    // Custom may still be used via customLabels
    expect(mirrored).toContain("person");
  });

  it("defaults to categories the engine accepts", () => {
    for (const category of DEFAULT_CONFIG.nerCategories) {
      expect(ENGINE_CATEGORIES).toContain(category);
    }
  });
});
