import { describe, expect, it, vi, beforeAll } from "vitest";
import type { Entity } from "../lib/types";
import type { PiiEntity } from "../lib/pii-engine";
import type { BridgeEntity } from "../lib/ner-bridge";

// `pipeline.ts` assigns `self.onmessage` at module scope (it's a worker entry
// point). Stub `self` before importing it dynamically so that assignment has
// something to land on under Node/vitest, where `self` doesn't exist. A static
// `import` would run pipeline.ts's module body before this stub takes effect.
let renderAnnotatedMarkdown: typeof import("./pipeline").renderAnnotatedMarkdown;
let filterExportableEntities: typeof import("./pipeline").filterExportableEntities;
let selectNerBridge: typeof import("./pipeline").selectNerBridge;

beforeAll(async () => {
  (globalThis as { self?: unknown }).self = globalThis;
  ({ renderAnnotatedMarkdown, filterExportableEntities, selectNerBridge } =
    await import("./pipeline"));
});

function piiEntity(overrides: Partial<PiiEntity>): PiiEntity {
  return {
    category: "iban",
    text: "",
    start: 0,
    end: 0,
    confidence: 1,
    source: "regex",
    format_preserving: true,
    redact_template: "[IBAN:****]",
    ...overrides,
  };
}

function entity(overrides: Partial<Entity>): Entity {
  return {
    name: "",
    type: "organization",
    slug: "",
    count: 1,
    spans: [],
    ...overrides,
  };
}

describe("renderAnnotatedMarkdown (Track F4)", () => {
  it("links a non-overlapping entity span normally", () => {
    const markdown = "Acme SAS acquired Beta SARL.";
    const acme = entity({
      name: "Acme SAS",
      type: "organization",
      slug: "acme-sas",
      spans: [{ start: 0, end: 8 }],
    });

    const result = renderAnnotatedMarkdown(markdown, [acme], []);

    expect(result).toBe(
      "[Acme SAS](#entity-organization-acme-sas) acquired Beta SARL.",
    );
  });

  it("redacts a PII span with no overlapping entity link", () => {
    const markdown = "IBAN FR7630006000011234567890189.";
    const iban = piiEntity({
      text: "FR7630006000011234567890189",
      start: 5,
      end: 32,
      redact_template: "[IBAN:****]",
    });

    const result = renderAnnotatedMarkdown(markdown, [], [iban]);

    expect(result).toBe("IBAN [IBAN:****].");
  });

  it("drops the entity link instead of double-rewriting when a PII span overlaps it — the L6 corruption bug", () => {
    // Reproduces the manually-observed failure: a misclassified NER entity (type
    // "phone") whose name/slug is the same digit run a PII span also matches. The
    // old two-pass pipeline (link first, then regex-redact the linked markdown)
    // matched that digit run twice — once in the link's visible text, once inside
    // its anchor target — producing a corrupted link. Detecting PII against the
    // original text and merging spans before a single splice must not do that.
    const markdown = "Card number 4111111111111111 on file.";
    const misclassifiedPhone = entity({
      name: "4111111111111111",
      type: "phone",
      slug: "4111111111111111",
      spans: [{ start: 12, end: 28 }],
    });
    const card = piiEntity({
      text: "4111111111111111",
      start: 12,
      end: 28,
      category: "credit_card",
      redact_template: "[CARD:****]",
    });

    const result = renderAnnotatedMarkdown(
      markdown,
      [misclassifiedPhone],
      [card],
    );

    expect(result).toBe("Card number [CARD:****] on file.");
    expect(result).not.toContain("entity-phone");
    expect(result).not.toContain("4111111111111111");
  });

  it("splices multiple non-overlapping entity and PII spans in one pass", () => {
    const markdown = "Contact Jean Dupont, IBAN FR7630006000011234567890189.";
    const jean = entity({
      name: "Jean Dupont",
      type: "person",
      slug: "jean-dupont",
      spans: [{ start: 8, end: 19 }],
    });
    const iban = piiEntity({
      text: "FR7630006000011234567890189",
      start: 26,
      end: 53,
      redact_template: "[IBAN:****]",
    });

    const result = renderAnnotatedMarkdown(markdown, [jean], [iban]);

    expect(result).toBe(
      "Contact [Jean Dupont](#entity-person-jean-dupont), IBAN [IBAN:****].",
    );
  });

  it("leaves entity links untouched when redaction findings are withheld (scan-only mode)", () => {
    const markdown = "Card number 4111111111111111 on file.";
    const misclassifiedPhone = entity({
      name: "4111111111111111",
      type: "phone",
      slug: "4111111111111111",
      spans: [{ start: 12, end: 28 }],
    });

    // The caller passes `[]` for findings when `redactPiiInOutput` is off — mirrors
    // `processFile`'s `config.redactPiiInOutput ? piiFindings : []`.
    const result = renderAnnotatedMarkdown(markdown, [misclassifiedPhone], []);

    expect(result).toBe(
      "Card number [4111111111111111](#entity-phone-4111111111111111) on file.",
    );
  });
});

describe("filterExportableEntities (Track A2)", () => {
  it("drops an entity whose span overlaps a PII finding when output is being redacted", () => {
    const misclassifiedPhone = entity({
      name: "4111111111111111",
      type: "phone",
      slug: "4111111111111111",
      spans: [{ start: 12, end: 28 }],
    });
    const card = piiEntity({ start: 12, end: 28, redact_template: "[CARD:****]" });

    const result = filterExportableEntities(
      [misclassifiedPhone],
      [card],
      true,
    );

    expect(result).toEqual([]);
  });

  it("keeps a non-overlapping entity when output is being redacted", () => {
    const jean = entity({
      name: "Jean Dupont",
      type: "person",
      slug: "jean-dupont",
      spans: [{ start: 8, end: 19 }],
    });
    const card = piiEntity({ start: 30, end: 46, redact_template: "[CARD:****]" });

    const result = filterExportableEntities([jean], [card], true);

    expect(result).toEqual([jean]);
  });

  it("drops the whole entity if any one of several mentions overlaps, not just that span", () => {
    const jean = entity({
      name: "Jean Dupont",
      type: "person",
      slug: "jean-dupont",
      spans: [
        { start: 0, end: 11 },
        { start: 50, end: 61 },
      ],
    });
    // Overlaps only the second mention.
    const finding = piiEntity({ start: 55, end: 58, redact_template: "[X:*]" });

    const result = filterExportableEntities([jean], [finding], true);

    expect(result).toEqual([]);
  });

  it("keeps everything untouched in scan-only mode, even with overlapping findings", () => {
    const misclassifiedPhone = entity({
      name: "4111111111111111",
      type: "phone",
      slug: "4111111111111111",
      spans: [{ start: 12, end: 28 }],
    });
    const card = piiEntity({ start: 12, end: 28, redact_template: "[CARD:****]" });

    const result = filterExportableEntities(
      [misclassifiedPhone],
      [card],
      false,
    );

    expect(result).toEqual([misclassifiedPhone]);
  });
});

describe("selectNerBridge (Track B1/B2)", () => {
  it("falls back to extractEntities (compromise.js) when no neural runtime loaded", async () => {
    const bridge = selectNerBridge(null);
    const frenchFixture = "Maître Jean Dupont a signé pour Acme SAS.";

    // Same baseline gap as lib/ner-bridge.test.ts: without the neural runtime,
    // the fallback still doesn't resolve the French fixture correctly. This
    // pins the fallback to the exact function `extractEntities`, not just a
    // function with the same signature.
    const persons = await bridge(frenchFixture, ["person"]);
    expect(persons.map((e) => e.text)).not.toContain("Jean Dupont");
  });

  it("calls the neural runtime's detect() and passes categories through opts when a runtime is loaded", async () => {
    const detected: BridgeEntity[] = [
      {
        category: "person",
        text: "Jean Dupont",
        start: 8,
        end: 19,
        confidence: 0.97,
      },
      {
        category: "organization",
        text: "Acme SAS",
        start: 33,
        end: 41,
        confidence: 0.95,
      },
    ];
    const detect = vi.fn().mockResolvedValue(detected);
    const mockRuntime = { detect } as unknown as Parameters<
      typeof selectNerBridge
    >[0];

    const bridge = selectNerBridge(mockRuntime);
    const frenchFixture = "Maître Jean Dupont a signé pour Acme SAS.";
    const result = await bridge(frenchFixture, ["person", "organization"]);

    expect(detect).toHaveBeenCalledWith(frenchFixture, {
      categories: ["person", "organization"],
    });
    expect(result.map((e) => e.text)).toEqual(["Jean Dupont", "Acme SAS"]);
    expect(result.map((e) => e.category)).toEqual(["person", "organization"]);
  });
});
