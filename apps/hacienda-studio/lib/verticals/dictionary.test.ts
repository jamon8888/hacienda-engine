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
});

/**
 * Track D1: `worker/pipeline.ts` used to build this dictionary from a
 * hardcoded `["m&a", "financial_services", "shared"]` list, so
 * `config.enabledVerticals` — the "Enabled Verticals" checkbox group in
 * `ConfigPanel.svelte` — was dead config; every taxonomy loaded regardless of
 * what the user unchecked. `processFiles` now builds the dictionary from
 * `config.enabledVerticals` directly, so these assert the one thing that
 * makes the toggle load-bearing: excluding a vertical here must make its
 * terms unresolvable, not merely reorder them.
 */
describe("VerticalDictionary built from a subset of enabledVerticals (Track D1)", () => {
  it("does not resolve m&a terms when only financial_services is enabled", async () => {
    const taxonomy = await loadVerticalTaxonomy("financial_services");
    const dictionary = new VerticalDictionary([taxonomy]);

    expect(dictionary.lookup("target_company")).toBeNull();
    expect(dictionary.lookup("portfolio_company")).toMatchObject({
      vertical: "financial_services",
    });
  });

  it("does not resolve financial_services terms when only m&a is enabled", async () => {
    const taxonomy = await loadVerticalTaxonomy("m&a");
    const dictionary = new VerticalDictionary([taxonomy]);

    // `portfolio_company` is listed in both taxonomies (PE-backed M&A deals), so it
    // is not a valid financial_services-only probe here — use `carried_interest`,
    // which only m&a's alias list (not its own entityTypes) ever points at.
    expect(dictionary.lookup("carried_interest")).toBeNull();
    expect(dictionary.lookup("target_company")).toMatchObject({
      vertical: "m&a",
    });
  });
});
