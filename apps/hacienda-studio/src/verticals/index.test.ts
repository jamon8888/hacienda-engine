import { loadVerticalTaxonomy, VerticalTaxonomy } from "./index";

test("loads M&A taxonomy with entity types and relationships", () => {
  const taxonomy = loadVerticalTaxonomy("m&a");
  expect(taxonomy.vertical).toBe("m&a");
  expect(taxonomy.entityTypes).toContain("target_company");
  expect(taxonomy.entityTypes).toContain("earnout");
  expect(taxonomy.relationships).toContain("acquirer_of");
});

test("loads Financial Services taxonomy", () => {
  const taxonomy = loadVerticalTaxonomy("financial_services");
  expect(taxonomy.vertical).toBe("financial_services");
  expect(taxonomy.entityTypes).toContain("fund");
  expect(taxonomy.entityTypes).toContain("carried_interest");
  expect(taxonomy.relationships).toContain("invests_in");
});

test("loads shared taxonomy", () => {
  const taxonomy = loadVerticalTaxonomy("shared");
  expect(taxonomy.vertical).toBe("shared");
  expect(taxonomy.entityTypes).toContain("person");
  expect(taxonomy.entityTypes).toContain("organization");
});
