import { describe, expect, it, vi, beforeAll } from "vitest";
import type { Entity } from "../lib/types";
import type { PiiEntity } from "../lib/pii-engine";
import type { BridgeEntity } from "../lib/ner-bridge";
import { BatchEntityRegistry, type RegistryEntity } from "../lib/registry";

// `pipeline.ts` assigns `self.onmessage` at module scope (it's a worker entry
// point). Stub `self` before importing it dynamically so that assignment has
// something to land on under Node/vitest, where `self` doesn't exist. A static
// `import` would run pipeline.ts's module body before this stub takes effect.
let renderAnnotatedMarkdown: typeof import("./pipeline").renderAnnotatedMarkdown;
let filterExportableEntities: typeof import("./pipeline").filterExportableEntities;
let selectNerBridge: typeof import("./pipeline").selectNerBridge;
let relativeEntityLink: typeof import("./pipeline").relativeEntityLink;
let buildEntityFile: typeof import("./pipeline").buildEntityFile;
let buildGlossaryIndex: typeof import("./pipeline").buildGlossaryIndex;
let assignDocId: typeof import("./pipeline").assignDocId;

beforeAll(async () => {
  (globalThis as { self?: unknown }).self = globalThis;
  ({
    renderAnnotatedMarkdown,
    filterExportableEntities,
    selectNerBridge,
    relativeEntityLink,
    buildEntityFile,
    buildGlossaryIndex,
    assignDocId,
  } = await import("./pipeline"));
});

function registryEntity(overrides: Partial<RegistryEntity>): RegistryEntity {
  return {
    id: "ent-001",
    canonical_name: "",
    display_name: "",
    type: "organization",
    slug: "",
    vertical: "shared",
    roles: [],
    aliases: [],
    source_documents: [],
    mention_count: 1,
    vertical_metadata: {},
    first_seen: "2026-07-30T00:00:00.000Z",
    last_seen: "2026-07-30T00:00:00.000Z",
    ...overrides,
  };
}

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

    const result = renderAnnotatedMarkdown(
      markdown,
      [acme],
      [],
      "documents/note.md",
    );

    expect(result).toBe(
      "[Acme SAS](../entities/organization-acme-sas.md) acquired Beta SARL.",
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

    const result = renderAnnotatedMarkdown(
      markdown,
      [],
      [iban],
      "documents/note.md",
    );

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
      "documents/note.md",
    );

    expect(result).toBe("Card number [CARD:****] on file.");
    expect(result).not.toContain("entities/phone");
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

    const result = renderAnnotatedMarkdown(
      markdown,
      [jean],
      [iban],
      "documents/note.md",
    );

    expect(result).toBe(
      "Contact [Jean Dupont](../entities/person-jean-dupont.md), IBAN [IBAN:****].",
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
    const result = renderAnnotatedMarkdown(
      markdown,
      [misclassifiedPhone],
      [],
      "documents/note.md",
    );

    expect(result).toBe(
      "Card number [4111111111111111](../entities/phone-4111111111111111.md) on file.",
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

describe("filterExportableEntities pseudonymize retention (Track A2 / Task 3)", () => {
  it("retains an entity whose overlapping finding carries a real pseudonym token, rekeyed on it", () => {
    const jean = entity({
      name: "Jean Dupont",
      type: "person",
      slug: "jean-dupont",
      spans: [{ start: 0, end: 11 }],
    });
    const finding = piiEntity({
      category: "person",
      start: 0,
      end: 11,
      redact_template: "[PERSON:session:MFRGG2LTMVRXEZLU]",
    });

    const [result] = filterExportableEntities([jean], [finding], true);

    expect(result).toBeDefined();
    expect(result.name).toBe("[PERSON:session:MFRGG2LTMVRXEZLU]");
    // The slug must not be derivable back to "Jean Dupont" — it's a hash of the token,
    // not of the real name, which is the whole point of this task.
    expect(result.slug).not.toBe("jean-dupont");
    expect(result.slug).toMatch(/^[0-9a-f]{8}$/);
  });

  it("still drops the entity when pseudonymize has no key (mask-shaped fallback template)", () => {
    // `worker/pipeline.ts`'s pseudonymize branch leaves `redact_template` at whatever
    // `process()`/`process_with_model_entities` already produced when `pseudonymKeyHex`
    // is null — a mask-shaped template, not a token. This is the degradation case
    // `AppConfig.redactionMode`'s doc comment describes.
    const jean = entity({
      name: "Jean Dupont",
      type: "person",
      slug: "jean-dupont",
      spans: [{ start: 0, end: 11 }],
    });
    const finding = piiEntity({
      category: "person",
      start: 0,
      end: 11,
      redact_template: "[PERSON]",
    });

    const result = filterExportableEntities([jean], [finding], true);

    expect(result).toEqual([]);
  });

  it("drops the entity if any one of several overlapping findings lacks a real token", () => {
    const jean = entity({
      name: "Jean Dupont",
      type: "person",
      slug: "jean-dupont",
      spans: [
        { start: 0, end: 11 },
        { start: 50, end: 61 },
      ],
    });
    const tokenFinding = piiEntity({
      category: "person",
      start: 0,
      end: 11,
      redact_template: "[PERSON:session:MFRGG2LTMVRXEZLU]",
    });
    const maskFinding = piiEntity({
      category: "person",
      start: 50,
      end: 61,
      redact_template: "[PERSON]",
    });

    const result = filterExportableEntities([jean], [tokenFinding, maskFinding], true);

    expect(result).toEqual([]);
  });

  it("never writes a surface name into a pseudonymized bundle: entity file, glossary, and registry all key on the token", async () => {
    const jean = entity({
      name: "Jean Dupont",
      type: "person",
      slug: "jean-dupont",
      spans: [{ start: 0, end: 11 }],
    });
    const acme = entity({
      name: "Acme SAS",
      type: "organization",
      slug: "acme-sas",
      spans: [{ start: 21, end: 29 }],
    });
    const findings = [
      piiEntity({
        category: "person",
        start: 0,
        end: 11,
        redact_template: "[PERSON:session:MFRGG2LTMVRXEZLU]",
      }),
      piiEntity({
        category: "organization",
        start: 21,
        end: 29,
        redact_template: "[ORGANIZATION:session:NBSWY3DPEB3W64TM]",
      }),
    ];

    const results = filterExportableEntities([jean, acme], findings, true);
    const registry = new BatchEntityRegistry();
    // `Promise.all`, not a plain `.map` (each entity is distinct — jean/acme never share
    // a dedup key — so there's no merge race here, but `buildGlossaryIndex` below reads
    // `registry.getEntities()`, which is empty until every `addEntity` call has actually
    // resolved and reached `this.entities.set(...)`).
    const registered = await Promise.all(
      results.map((r) =>
        registry.addEntity(
          { name: r.name, type: r.type, slug: r.slug, count: r.count, spans: r.spans },
          { vertical: "shared" },
          "doc-001",
        ),
      ),
    );
    const glossary = buildGlossaryIndex(registry.getEntities());
    const entityFiles = registered.map((r) =>
      buildEntityFile(r, [registry.getBatchId() + "/doc"]),
    );

    for (const surfaceName of ["Jean Dupont", "Acme SAS"]) {
      for (const result of results) {
        expect(result.name).not.toContain(surfaceName);
        expect(result.slug).not.toContain(surfaceName.toLowerCase().replace(/\s+/g, "-"));
      }
      expect(glossary).not.toContain(surfaceName);
      for (const file of entityFiles) {
        expect(file).not.toContain(surfaceName);
      }
    }
  });

  it("gives the same real-world entity the same token across two documents, so cross-document dedup keys on it for free", async () => {
    const { deriveKeyHex, mintToken } = await import("../lib/pseudonymize");
    const keyHex = await deriveKeyHex("correct horse battery staple", "session");

    // Simulates `worker/pipeline.ts`'s `processFiles`: one key derived once for the whole
    // batch, `mintToken` called independently per document against the same real text.
    const tokenInDoc1 = await mintToken("person", "Jean Dupont", "session", keyHex);
    const tokenInDoc2 = await mintToken("person", "Jean Dupont", "session", keyHex);
    expect(tokenInDoc1).toBe(tokenInDoc2); // the SIV determinism this task's fix relies on

    const jeanInDoc1 = entity({
      name: "Jean Dupont",
      type: "person",
      slug: "jean-dupont",
      spans: [{ start: 0, end: 11 }],
    });
    const jeanInDoc2 = entity({
      name: "Jean Dupont",
      type: "person",
      slug: "jean-dupont",
      spans: [{ start: 30, end: 41 }],
    });
    const findingInDoc1 = piiEntity({
      category: "person",
      start: 0,
      end: 11,
      redact_template: tokenInDoc1,
    });
    const findingInDoc2 = piiEntity({
      category: "person",
      start: 30,
      end: 41,
      redact_template: tokenInDoc2,
    });

    const [resultDoc1] = filterExportableEntities([jeanInDoc1], [findingInDoc1], true);
    const [resultDoc2] = filterExportableEntities([jeanInDoc2], [findingInDoc2], true);

    // Same name -> same token -> same name/slug on the exported entity -> the registry's
    // existing `${normalizedName}|${type}|${vertical}` dedup key merges them into one
    // entity with both documents in `source_documents`, with no code change to the
    // registry itself required.
    expect(resultDoc1.name).toBe(resultDoc2.name);
    expect(resultDoc1.slug).toBe(resultDoc2.slug);

    const registry = new BatchEntityRegistry();
    // Awaited sequentially, not fired concurrently: both calls share the same dedup key
    // (asserted above), and `addEntity` only records a new entity's id in `entityKeyMap`
    // after an internal `await` — two un-awaited calls for the same entity would both see
    // no existing entry and both create one, silently losing the merge this test exists to
    // prove (the second `.set()` would overwrite the first's `source_documents`, not add
    // to it).
    await registry.addEntity(
      { name: resultDoc1.name, type: resultDoc1.type, slug: resultDoc1.slug, count: 1, spans: resultDoc1.spans },
      { vertical: "shared" },
      "doc-001",
    );
    const merged = await registry.addEntity(
      { name: resultDoc2.name, type: resultDoc2.type, slug: resultDoc2.slug, count: 1, spans: resultDoc2.spans },
      { vertical: "shared" },
      "doc-002",
    );
    expect(merged.source_documents).toEqual(["doc-001", "doc-002"]);
  });

  it("leaves mask, hash, and remove entity sets unaffected by this task's change", () => {
    // Unit-level substitute for a byte-level bundle diff — Task 0 could not produce
    // reference bundles on this host (no dev server / model available). This asserts the
    // narrower but still meaningful property: for every mode except pseudonymize-with-a-
    // key, `filterExportableEntities`'s decision per entity is identical to what it was
    // before this task (drop on any overlap, keep otherwise) — `redact_template` shapes
    // from mask (`[EMAIL]`), hash (`#email:1a2b…`), and remove (`""`) all fail
    // `looksLikePseudonymToken`, so the new "retain and rekey" branch is never taken.
    const jean = entity({
      name: "Jean Dupont",
      type: "person",
      slug: "jean-dupont",
      spans: [{ start: 0, end: 11 }],
    });
    const templates = ["[PERSON]", "#person:1a2b3c4d5e6f7890", ""];

    for (const redact_template of templates) {
      const finding = piiEntity({ category: "person", start: 0, end: 11, redact_template });
      const result = filterExportableEntities([jean], [finding], true);
      expect(result).toEqual([]);
    }
  });
});

describe("relativeEntityLink (Track I2)", () => {
  const acme = { type: "organization", slug: "acme-sas" };

  it("one level up for a document at the documents/ root", () => {
    expect(relativeEntityLink("documents/note.md", acme)).toBe(
      "../entities/organization-acme-sas.md",
    );
  });

  it("adds one more '../' per nested source directory", () => {
    expect(relativeEntityLink("documents/sub/note.md", acme)).toBe(
      "../../entities/organization-acme-sas.md",
    );
    expect(relativeEntityLink("documents/a/b/note.md", acme)).toBe(
      "../../../entities/organization-acme-sas.md",
    );
  });
});

describe("buildEntityFile (Track I2)", () => {
  it("renders type/vertical/roles/mentions and a backlink per document", () => {
    const entity = registryEntity({
      display_name: "Acme SAS",
      type: "organization",
      vertical: "m&a",
      roles: ["acquirer"],
      mention_count: 2,
      source_documents: ["doc-001", "doc-002"],
    });

    const md = buildEntityFile(entity, [
      "documents/contract.md",
      "documents/sub/note.md",
    ]);

    expect(md).toContain("# Acme SAS");
    expect(md).toContain("**Type:** Organization");
    expect(md).toContain("**Vertical:** m&a");
    expect(md).toContain("**Roles:** acquirer");
    expect(md).toContain("**Mentions:** 2 across 2 documents");
    expect(md).toContain("[contract.md](../documents/contract.md)");
    expect(md).toContain("[sub/note.md](../documents/sub/note.md)");
  });

  it("omits optional fields that are empty rather than printing them blank", () => {
    const entity = registryEntity({ display_name: "Jean Dupont", type: "person" });
    const md = buildEntityFile(entity, []);
    expect(md).not.toContain("**Sector:**");
    expect(md).not.toContain("**Roles:**");
    expect(md).not.toContain("**Aliases:**");
  });
});

describe("buildGlossaryIndex (Track I2)", () => {
  it("groups entities by type, alphabetically, with a link into entities/", () => {
    const md = buildGlossaryIndex([
      registryEntity({
        display_name: "Beta SARL",
        type: "organization",
        vertical: "m&a",
        mention_count: 1,
        source_documents: ["doc-001"],
        slug: "beta-sarl",
      }),
      registryEntity({
        display_name: "Jean Dupont",
        type: "person",
        mention_count: 3,
        source_documents: ["doc-001", "doc-002"],
        slug: "jean-dupont",
      }),
    ]);

    expect(md).toContain("# Glossary");
    // "Organization" sorts before "Person" alphabetically.
    expect(md.indexOf("## Organization")).toBeLessThan(md.indexOf("## Person"));
    expect(md).toContain(
      "[Beta SARL](entities/organization-beta-sarl.md) — m&a, mentioned 1 time across 1 document",
    );
    expect(md).toContain(
      "[Jean Dupont](entities/person-jean-dupont.md), mentioned 3 times across 2 documents",
    );
  });

  it("says so plainly when the batch found no entities", () => {
    expect(buildGlossaryIndex([])).toBe(
      "# Glossary\n\nNo entities were detected in this batch.\n",
    );
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

describe("assignDocId (Task 4, spec §8 step 4)", () => {
  function bytesOf(text: string): ArrayBuffer {
    return new TextEncoder().encode(text).buffer;
  }

  it("gives the same file bytes the same id across independent batches", async () => {
    const bytes = bytesOf("the quick brown fox");
    const idA = await assignDocId(bytes, new Set());
    const idB = await assignDocId(bytesOf("the quick brown fox"), new Set());

    expect(idA).toBe(idB);
    expect(idA).toMatch(/^doc-[0-9a-f]{12}$/);
  });

  it("gives different file bytes different ids", async () => {
    const idA = await assignDocId(bytesOf("file one"), new Set());
    const idB = await assignDocId(bytesOf("file two"), new Set());

    expect(idA).not.toBe(idB);
  });

  it("disambiguates two byte-identical files in the same batch rather than colliding", async () => {
    // `docId` doubles as a `docPaths` Map key — a silent collision here would let the
    // second duplicate's export path overwrite the first's, corrupting both files'
    // backlinks. `usedIds` is the same Set across both calls, simulating two duplicate
    // uploads in one `processFiles` batch.
    const usedIds = new Set<string>();
    const bytes = bytesOf("duplicate upload");

    const first = await assignDocId(bytes, usedIds);
    const second = await assignDocId(bytes, usedIds);

    expect(first).not.toBe(second);
    expect(second).toBe(`${first}-2`);

    const third = await assignDocId(bytes, usedIds);
    expect(third).toBe(`${first}-3`);
  });
});
