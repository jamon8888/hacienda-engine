import { describe, it, expect } from "vitest";
import { VerticalDictionary } from "./dictionary";
import { loadVerticalTaxonomy } from "./index";

async function buildDictionary(): Promise<VerticalDictionary> {
  const taxonomies = await Promise.all(
    ["m&a", "financial_services", "shared"].map(loadVerticalTaxonomy),
  );
  return new VerticalDictionary(taxonomies);
}

describe("VerticalDictionary", () => {
  it("resolves an entity type to its vertical", async () => {
    const dictionary = await buildDictionary();

    expect(dictionary.lookup("target_company")).toMatchObject({
      canonical: "target_company",
      vertical: "m&a",
    });
    expect(dictionary.lookup("carried_interest")).toMatchObject({
      vertical: "financial_services",
    });
  });

  it("resolves aliases to the canonical entity type", async () => {
    const dictionary = await buildDictionary();

    expect(dictionary.lookup("reps and warranties")?.canonical).toBe(
      "representation_and_warranty",
    );
    expect(dictionary.lookup("portco")?.canonical).toBe("portfolio_company");
  });

  it("ignores case and trailing punctuation", async () => {
    const dictionary = await buildDictionary();

    expect(dictionary.lookup("Target Company")?.canonical).toBe(
      "target_company",
    );
    expect(dictionary.lookup("earnout,")?.canonical).toBe("earnout");
  });

  it("returns null for terms outside every taxonomy", async () => {
    const dictionary = await buildDictionary();

    expect(dictionary.lookup("croissant")).toBeNull();
  });

  /**
   * Regression: sector was derived as `taxonomy.sectors[0]`, which stamped
   * `technology` on every M&A entity and `undefined` on every shared one. That
   * fabricated value propagated into the exported knowledge graph as fact.
   */
  it("does not fabricate a sector from the vertical's sector list", async () => {
    const dictionary = await buildDictionary();

    expect(dictionary.lookup("target_company")?.sector).toBeUndefined();
    expect(dictionary.lookup("fund")?.sector).toBeUndefined();
  });

  /**
   * Track D1: `worker/pipeline.ts`'s `processFiles` used to load all three
   * taxonomies unconditionally, ignoring `config.enabledVerticals` entirely —
   * the "Vertical NER" checkboxes in ConfigPanel.tsx did nothing. The fix
   * is `config.enabledVerticals.map(loadVerticalTaxonomy)` instead of a
   * hardcoded list; this asserts that restriction actually excludes terms
   * from a disabled vertical, which is the one thing a full pipeline e2e test
   * can't assert reliably (it would depend on the heuristic NER bridge
   * extracting a taxonomy term as a named entity in the first place).
   */
  it("excludes a vertical's terms when its taxonomy isn't in the loaded set", async () => {
    const taxonomies = await Promise.all(
      (["financial_services", "shared"] as const).map(loadVerticalTaxonomy),
    );
    const dictionary = new VerticalDictionary(taxonomies);

    expect(dictionary.lookup("target_company")).toBeNull();
    expect(dictionary.lookup("carried_interest")).toMatchObject({
      vertical: "financial_services",
    });
  });

  it("resolves nothing when the enabled-verticals selection is empty", async () => {
    const dictionary = new VerticalDictionary([]);

    expect(dictionary.lookup("target_company")).toBeNull();
    expect(dictionary.lookup("carried_interest")).toBeNull();
  });
});
