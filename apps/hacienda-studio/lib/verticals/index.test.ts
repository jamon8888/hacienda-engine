import { describe, it, expect } from "vitest";
import { loadVerticalTaxonomy } from "./index";

/**
 * Regression: taxonomies used to be fetched from /src/lib/verticals/*.yaml.
 * That URL did not resolve, the SPA fallback answered with index.html and
 * HTTP 200 so `response.ok` was true, and the parsed taxonomy came back with
 * no entityTypes — crashing the worker on the first file processed. Assert the
 * shape every caller depends on.
 */
describe("loadVerticalTaxonomy", () => {
  it.each(["m&a", "financial_services", "shared"])(
    "loads %s with populated entity types and relationships",
    async (vertical) => {
      const taxonomy = await loadVerticalTaxonomy(vertical);

      expect(taxonomy.vertical).toBe(vertical);
      expect(taxonomy.entityTypes.length).toBeGreaterThan(0);
      expect(taxonomy.relationships.length).toBeGreaterThan(0);
      expect(taxonomy.entityTypes.every((type) => type.length > 0)).toBe(true);
    },
  );

  it("parses the M&A taxonomy", async () => {
    const taxonomy = await loadVerticalTaxonomy("m&a");

    expect(taxonomy.entityTypes).toContain("target_company");
    expect(taxonomy.entityTypes).toContain("earnout");
    expect(taxonomy.relationships).toContain("acquirer_of");
    expect(taxonomy.sectors).toContain("technology");
  });

  it("parses the financial services taxonomy", async () => {
    const taxonomy = await loadVerticalTaxonomy("financial_services");

    expect(taxonomy.entityTypes).toContain("fund");
    expect(taxonomy.entityTypes).toContain("carried_interest");
    expect(taxonomy.relationships).toContain("invests_in");
  });

  it("parses the shared taxonomy", async () => {
    const taxonomy = await loadVerticalTaxonomy("shared");

    expect(taxonomy.entityTypes).toContain("person");
    expect(taxonomy.entityTypes).toContain("organization");
    expect(taxonomy.sectors).toEqual([]);
  });

  it("rejects an unknown vertical rather than returning an empty taxonomy", async () => {
    await expect(loadVerticalTaxonomy("does_not_exist")).rejects.toThrow(
      /Unknown vertical taxonomy/,
    );
  });
});
