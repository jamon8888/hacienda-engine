/**
 * Task 5.1 (spec §8 step 5, §4/§5.1.1): `CLAUDE.md`/`AGENTS.md` — the routing table and
 * redaction contract auto-loaded by their respective agent runtimes. These tests go
 * through the real `assembleZip`, not a hand-rolled stand-in, so "both members exist,
 * byte-identical" is verified against the actual zip a user downloads, not against an
 * assumption about how `assembleZip` wires the two files together.
 */
import { describe, it, expect } from "vitest";
import JSZip from "jszip";
import {
  assembleZip,
  buildGlossaryIndex,
  buildByTypeIndex,
  buildEntitiesJsonl,
  buildDocumentsJsonl,
  buildTimelineIndex,
  type ZipBatch,
} from "./zip-export";
import { BatchEntityRegistry, type RegistryEntity } from "./registry";
import { DEFAULT_CONFIG, type ProcessedFile } from "./types";

function registryEntity(overrides: Partial<RegistryEntity> = {}): RegistryEntity {
  return {
    id: "ent-x",
    canonical_name: "X",
    display_name: "X",
    type: "organization",
    slug: "x",
    vertical: "shared",
    roles: [],
    aliases: [],
    source_documents: ["doc-001"],
    mention_count: 1,
    vertical_metadata: {},
    first_seen: "",
    last_seen: "",
    ...overrides,
  };
}

function processedFile(overrides: Partial<ProcessedFile> = {}): ProcessedFile {
  return {
    name: "doc.md",
    markdown: "# doc\n\nSome content.",
    rawMarkdown: "Some content.",
    entities: [],
    piiFindings: [],
    frontmatter: {
      source: "doc.pdf",
      type: "pdf",
      processed: new Date().toISOString(),
      piiEntitiesFound: 0,
      entities: [],
    },
    ...overrides,
  };
}

async function buildBatch(configOverrides: Partial<typeof DEFAULT_CONFIG>): Promise<ZipBatch> {
  const registry = new BatchEntityRegistry();
  const docPaths = new Map([["doc-001", "documents/doc.md"]]);
  return {
    results: [processedFile()],
    registry,
    docPaths,
    config: { ...DEFAULT_CONFIG, ...configOverrides },
  };
}

async function extractMember(blob: Blob, name: string): Promise<string> {
  const zip = await JSZip.loadAsync(await blob.arrayBuffer());
  const file = zip.file(name);
  if (!file) throw new Error(`zip has no ${name} member`);
  return file.async("string");
}

describe("assembleZip: CLAUDE.md / AGENTS.md (Task 5.1)", () => {
  it("writes both files, byte-identical", async () => {
    const blob = await assembleZip(await buildBatch({}));
    const claudeMd = await extractMember(blob, "CLAUDE.md");
    const agentsMd = await extractMember(blob, "AGENTS.md");

    expect(claudeMd.length).toBeGreaterThan(0);
    expect(claudeMd).toBe(agentsMd);
  });

  it("mask mode does not claim tokens are stable identities", async () => {
    const blob = await assembleZip(
      await buildBatch({ redactPiiInOutput: true, redactionMode: "mask" }),
    );
    const claudeMd = await extractMember(blob, "CLAUDE.md");

    expect(claudeMd).toContain("mask");
    expect(claudeMd).not.toContain("stable, non-identifying");
  });

  it("pseudonymize mode claims token stability only when a real token is actually present", async () => {
    const registry = new BatchEntityRegistry();
    await registry.addEntity(
      { name: "[PERSON:session:MFRGG2LTMVRXEZLU]", type: "person", slug: "x", count: 1, spans: [] },
      { vertical: "shared" },
      "doc-001",
    );
    const batch: ZipBatch = {
      results: [processedFile()],
      registry,
      docPaths: new Map([["doc-001", "documents/doc.md"]]),
      config: { ...DEFAULT_CONFIG, redactPiiInOutput: true, redactionMode: "pseudonymize" },
    };

    const claudeMd = await extractMember(await assembleZip(batch), "CLAUDE.md");

    expect(claudeMd).toContain("stable, non-identifying");
  });

  it("pseudonymize mode does NOT claim token stability when no token-shaped entity exists (silent mask fallback)", async () => {
    // Simulates the degradation `AppConfig.pseudonymPassphrase`'s doc comment describes:
    // `redactionMode` says pseudonymize, but no passphrase was given, so nothing in the
    // registry actually carries a token — the contract must not overclaim in this case.
    const batch = await buildBatch({ redactPiiInOutput: true, redactionMode: "pseudonymize" });
    const claudeMd = await extractMember(await assembleZip(batch), "CLAUDE.md");

    expect(claudeMd).not.toContain("stable, non-identifying");
    expect(claudeMd).toContain("did not take effect");
  });

  it("hash mode describes digests, not tokens or masks", async () => {
    const blob = await assembleZip(
      await buildBatch({ redactPiiInOutput: true, redactionMode: "hash" }),
    );
    const claudeMd = await extractMember(blob, "CLAUDE.md");

    expect(claudeMd).toContain("keyed digest");
    expect(claudeMd).not.toContain("stable, non-identifying");
  });

  it("remove mode says content was deleted outright", async () => {
    const blob = await assembleZip(
      await buildBatch({ redactPiiInOutput: true, redactionMode: "remove" }),
    );
    const claudeMd = await extractMember(blob, "CLAUDE.md");

    expect(claudeMd).toContain("deleted outright");
  });

  it("scan-only (redactPiiInOutput false) says nothing was redacted", async () => {
    const blob = await assembleZip(await buildBatch({ redactPiiInOutput: false }));
    const claudeMd = await extractMember(blob, "CLAUDE.md");

    expect(claudeMd).toContain("scan-only mode");
  });
});

describe("buildGlossaryIndex top-N gating (Task 5.3)", () => {
  it("shows every entity and no 'Full index' pointer when under the cutoff", () => {
    const entities = [registryEntity({ display_name: "A" }), registryEntity({ display_name: "B" })];
    const md = buildGlossaryIndex(entities, 5);

    expect(md).toContain("Top 2 of 2");
    expect(md).not.toContain("Full index by type");
  });

  it("gates to topN by mention count and points to the by-type index when over the cutoff", () => {
    const entities = [
      registryEntity({ display_name: "Rare", mention_count: 1 }),
      registryEntity({ display_name: "Common", mention_count: 100 }),
      registryEntity({ display_name: "Medium", mention_count: 10 }),
    ];
    const md = buildGlossaryIndex(entities, 2);

    expect(md).toContain("Top 2 of 3");
    expect(md).toContain("Common");
    expect(md).toContain("Medium");
    expect(md).not.toContain("- [Rare]"); // dropped — lowest mention count, over the cutoff
    expect(md).toContain("Full index by type");
    expect(md).toContain("indexes/by-type/organization.md");
  });
});

describe("buildByTypeIndex (Task 5.3)", () => {
  it("links back to entities/ two directories up, matching indexes/by-type/'s real depth", () => {
    const md = buildByTypeIndex("organization", [registryEntity({ display_name: "Acme" })]);

    expect(md).toContain("(../../entities/organization-x.md)");
  });

  it("is not gated — every entity of the type appears, unlike GLOSSARY.md", () => {
    const entities = Array.from({ length: 60 }, (_, i) =>
      registryEntity({ display_name: `Entity ${i}`, slug: `e${i}` }),
    );
    const md = buildByTypeIndex("organization", entities);

    for (const e of entities) expect(md).toContain(e.display_name);
  });
});

describe("buildEntitiesJsonl / buildDocumentsJsonl (Task 5.3)", () => {
  it("emits one parseable JSON object per line for entities", () => {
    const entities = [
      registryEntity({ id: "ent-a", display_name: "A" }),
      registryEntity({ id: "ent-b", display_name: "B" }),
    ];
    const jsonl = buildEntitiesJsonl(entities);
    const lines = jsonl.split("\n");

    expect(lines).toHaveLength(2);
    for (const line of lines) expect(() => JSON.parse(line)).not.toThrow();
    expect(JSON.parse(lines[0]).id).toBe("ent-a");
  });

  it("emits one parseable JSON object per line for documents, cross-referencing docId", () => {
    const results = [
      { name: "a.md", entities: [{}, {}], frontmatter: { piiEntitiesFound: 2 } } as unknown as ProcessedFile,
    ];
    const docPaths = new Map([["doc-001", "documents/a.md"]]);
    const jsonl = buildDocumentsJsonl(results, docPaths);
    const record = JSON.parse(jsonl);

    expect(record.id).toBe("doc-001");
    expect(record.path).toBe("documents/a.md");
    expect(record.entity_count).toBe(2);
    expect(record.pii_entities_found).toBe(2);
  });
});

describe("buildTimelineIndex (Task 5.3)", () => {
  it("sorts ISO dates chronologically", () => {
    const entities = [
      registryEntity({ type: "date", display_name: "2024-06-01", slug: "d1" }),
      registryEntity({ type: "date", display_name: "2024-01-15", slug: "d2" }),
    ];
    const md = buildTimelineIndex(entities);

    expect(md.indexOf("2024-01-15")).toBeLessThan(md.indexOf("2024-06-01"));
  });

  it("puts non-ISO dates in a separate, explicitly-labelled unparsed section", () => {
    const entities = [
      registryEntity({ type: "date", display_name: "15 mars 2024", slug: "d1" }),
      registryEntity({ type: "date", display_name: "2024-01-15", slug: "d2" }),
    ];
    const md = buildTimelineIndex(entities);

    expect(md).toContain("Other dates (format not recognised");
    const unparsedSectionStart = md.indexOf("Other dates");
    expect(md.indexOf("15 mars 2024")).toBeGreaterThan(unparsedSectionStart);
  });

  it("ignores non-date entities entirely", () => {
    const entities = [registryEntity({ type: "person", display_name: "Jean Dupont" })];
    const md = buildTimelineIndex(entities);

    expect(md).toContain("No date entities");
    expect(md).not.toContain("Jean Dupont");
  });
});

describe("assembleZip: indexes/, _index/ (Task 5.3 integration)", () => {
  it("writes by-type indexes, timeline, and both jsonl sidecars into the real zip", async () => {
    const registry = new BatchEntityRegistry();
    await registry.addEntity(
      { name: "Acme SAS", type: "organization", slug: "x", count: 1, spans: [] },
      { vertical: "shared" },
      "doc-001",
    );
    const batch: ZipBatch = {
      results: [processedFile()],
      registry,
      docPaths: new Map([["doc-001", "documents/doc.md"]]),
      config: { ...DEFAULT_CONFIG },
    };

    const zip = await JSZip.loadAsync(await (await assembleZip(batch)).arrayBuffer());

    expect(zip.file("indexes/by-type/organization.md")).not.toBeNull();
    expect(zip.file("indexes/timeline.md")).not.toBeNull();
    expect(zip.file("_index/entities.jsonl")).not.toBeNull();
    expect(zip.file("_index/documents.jsonl")).not.toBeNull();

    const entitiesJsonl = await zip.file("_index/entities.jsonl")!.async("string");
    expect(() => JSON.parse(entitiesJsonl.trim())).not.toThrow();
  });
});
