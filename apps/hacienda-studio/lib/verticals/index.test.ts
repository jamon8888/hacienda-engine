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
  it.each(["m&a", "financial_services", "business_law", "shared"])(
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

  it("parses the business law taxonomy", async () => {
    const taxonomy = await loadVerticalTaxonomy("business_law");

    expect(taxonomy.entityTypes).toContain("contracting_party");
    expect(taxonomy.entityTypes).toContain("indemnification_clause");
    expect(taxonomy.relationships).toContain("governed_by");
  });

  /**
   * VerticalDictionary's lookup map (dictionary.ts) is a flat Map keyed by
   * lowercased entity type — the last-loaded taxonomy silently wins a shared
   * key. business_law.yaml intentionally omits governing_law/jurisdiction/
   * regulator because m&a.yaml/shared.yaml already own them (see the review
   * note in superpowers/specs/2026-07-29-business-law-gliner2-lora-design.md
   * §4); this guards against a future edit silently reintroducing that.
   */
  it("does not let business_law redeclare an entity type owned by another vertical", async () => {
    const [ma, financialServices, shared, businessLaw] = await Promise.all(
      ["m&a", "financial_services", "shared", "business_law"].map(
        loadVerticalTaxonomy,
      ),
    );
    const owned = new Map<string, string>();
    for (const taxonomy of [ma, financialServices, shared]) {
      for (const type of taxonomy.entityTypes) {
        owned.set(type, taxonomy.vertical);
      }
    }

    for (const type of businessLaw.entityTypes) {
      expect(
        owned.has(type),
        `"${type}" is already owned by vertical "${owned.get(type)}"; business_law must not redeclare it`,
      ).toBe(false);
    }
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
