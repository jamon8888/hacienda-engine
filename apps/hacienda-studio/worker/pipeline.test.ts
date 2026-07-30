import { describe, expect, it, beforeAll } from "vitest";
import type { Entity } from "../lib/types";
import type { PiiEntity } from "../lib/pii-engine";

// `pipeline.ts` assigns `self.onmessage` at module scope (it's a worker entry
// point). Stub `self` before importing it dynamically so that assignment has
// something to land on under Node/vitest, where `self` doesn't exist. A static
// `import` would run pipeline.ts's module body before this stub takes effect.
let renderAnnotatedMarkdown: typeof import("./pipeline").renderAnnotatedMarkdown;

beforeAll(async () => {
  (globalThis as { self?: unknown }).self = globalThis;
  ({ renderAnnotatedMarkdown } = await import("./pipeline"));
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
      "[Acme SAS](entity:organization/acme-sas) acquired Beta SARL.",
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
    // its `entity:` slug — producing
    // `[[CARD:****]](entity:phone/[CARD:****])`. Detecting PII against the
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
    expect(result).not.toContain("entity:phone");
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
      "Contact [Jean Dupont](entity:person/jean-dupont), IBAN [IBAN:****].",
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
      "Card number [4111111111111111](entity:phone/4111111111111111) on file.",
    );
  });
});
