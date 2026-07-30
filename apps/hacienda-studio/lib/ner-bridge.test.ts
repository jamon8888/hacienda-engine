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

  /**
   * Track D2: `PHONE_PATTERN` only matched US-style 3-3-4 grouping
   * (`555-123-4567`); a French number grouped in pairs from a leading 0
   * (`01 23 45 67 89`, present in `CONTRACT` above since this file's first
   * version) was silently never matched by any test — the exact way a dead
   * capability survives, per Track D's own reasoning.
   */
  it("finds a French-grouped phone number", async () => {
    const phones = await extractEntities(CONTRACT, ["phone"]);
    expect(phones.map((e) => e.text)).toEqual(["01 23 45 67 89"]);
  });

  it("returns nothing for categories that were not requested", async () => {
    const entities = await extractEntities(CONTRACT, []);
    expect(entities).toEqual([]);
  });
});

/**
 * Track D2 (supports B2): a French-language fixture, distinct from `CONTRACT`
 * above because it specifically probes the categories that need `compromise`,
 * not just the regex-based ones. `compromise` is English-only — its output on
 * this text is not a fixture bug, it is the documented reason Track B exists
 * (swap in xberg-wasm's multilingual `NerModel`). These assertions describe
 * *today's* actual, known-poor behavior, not the desired one:
 * person/organization names are the one category this bridge cannot get
 * right for French text no matter how the regex patterns are tuned.
 *
 * When B2 lands, this whole describe block's assertions should flip to
 * positive ones (organizations contains "Acme SAS", not the reverse) against
 * whatever bridge B2 wires in — a failure here after that point means this
 * fixture needs re-checking, not that something regressed.
 */
describe("French-language documents (Track D2, supports B2)", () => {
  const FRENCH_DEAL = [
    "Le 12 mars 2024, Maître Jean Dupont a représenté Acme SAS dans l'acquisition de Beta SARL.",
    "La société cible est basée à Paris. Contact: jean.dupont@cabinet-exemple.fr, tel 01 23 45 67 89.",
  ].join("\n");

  it("finds structured entities language-independently: date, email, phone", async () => {
    const entities = await extractEntities(FRENCH_DEAL, [
      "date",
      "email",
      "phone",
    ]);

    expect(
      entities.filter((e) => e.category === "date").map((e) => e.text),
    ).toEqual(["12 mars 2024"]);
    expect(
      entities.filter((e) => e.category === "email").map((e) => e.text),
    ).toEqual(["jean.dupont@cabinet-exemple.fr"]);
    expect(
      entities.filter((e) => e.category === "phone").map((e) => e.text),
    ).toEqual(["01 23 45 67 89"]);
  });

  it("does not reliably extract French person/organization names yet — the gap Track B closes", async () => {
    const entities = await extractEntities(FRENCH_DEAL, [
      "person",
      "organization",
    ]);
    const organizations = entities
      .filter((e) => e.category === "organization")
      .map((e) => e.text);
    const persons = entities
      .filter((e) => e.category === "person")
      .map((e) => e.text);

    expect(organizations).not.toContain("Acme SAS");
    expect(organizations).not.toContain("Beta SARL");
    expect(persons).not.toContain("Jean Dupont");
    // What it does instead: misreads the person as an organization, and
    // treats the generic noun phrase "La société" ("the company") as a named
    // one — both wrong, and both exactly why B2 is a model swap, not a tuning
    // pass on this bridge.
    expect(organizations).toContain("Jean Dupont");
  });
});
