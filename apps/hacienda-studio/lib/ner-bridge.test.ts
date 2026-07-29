import { describe, expect, it } from "vitest";
import { extractEntities } from "./ner-bridge";
import { DEFAULT_CONFIG, type NerCategory } from "./types";

const ALL_CATEGORIES: NerCategory[] = [
  "person",
  "organization",
  "location",
  "date",
  "money",
  "percent",
  "email",
  "phone",
  "url",
];

const CONTRACT = [
  "Le 12 mars 2024, Acme SAS a acquis Beta SARL.",
  "Contact: jean.dupont@cabinet-exemple.fr, tel 01 23 45 67 89.",
  "Voir https://exemple.fr/dossier pour le prix de 4.2M EUR (15% de prime).",
].join("\n");

describe("NER bridge", () => {
  /**
   * Two categories shipped broken because nothing exercised this function: the
   * bridge emitted the label `phone_number`, which is outside the engine's
   * vocabulary and made it reject the entire result, and `date` called
   * `doc.dates()`, which only exists with the compromise-dates plugin
   * installed. Both surfaced only as "Unknown error" against the document.
   */
  it("handles every category the UI can enable without throwing", async () => {
    for (const category of ALL_CATEGORIES) {
      await expect(extractEntities(CONTRACT, [category])).resolves.toBeTypeOf(
        "object",
      );
    }
  });

  it("labels entities with the engine's vocabulary", async () => {
    const entities = await extractEntities(CONTRACT, ALL_CATEGORIES);
    for (const entity of entities) {
      expect(ALL_CATEGORIES).toContain(entity.category);
    }
  });

  it("reports offsets that point at the mention in the source text", async () => {
    const entities = await extractEntities(CONTRACT, ALL_CATEGORIES);
    expect(entities.length).toBeGreaterThan(0);
    for (const entity of entities) {
      expect(CONTRACT.slice(entity.start, entity.end)).toBe(entity.text);
    }
  });

  it("finds the email, url and French date", async () => {
    const entities = await extractEntities(CONTRACT, DEFAULT_CONFIG.nerCategories);
    const emails = entities.filter((e) => e.category === "email");
    expect(emails.map((e) => e.text)).toEqual([
      "jean.dupont@cabinet-exemple.fr",
    ]);

    const dates = await extractEntities(CONTRACT, ["date"]);
    expect(dates.map((e) => e.text)).toEqual(["12 mars 2024"]);

    const urls = await extractEntities(CONTRACT, ["url"]);
    expect(urls.map((e) => e.text)).toEqual(["https://exemple.fr/dossier"]);
  });

  it("returns nothing for categories that were not requested", async () => {
    const entities = await extractEntities(CONTRACT, []);
    expect(entities).toEqual([]);
  });
});
