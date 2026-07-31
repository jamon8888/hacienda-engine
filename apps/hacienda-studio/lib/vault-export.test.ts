import { describe, expect, it } from "vitest";
import { vaultRelativeLink, buildEntityMarkdown, buildGlossaryMarkdown, buildReadme } from "./vault-export";
import type { RegistryEntity } from "./registry";

function registryEntity(overrides: Partial<RegistryEntity>): RegistryEntity {
  return {
    id: "ent-001",
    slug: "acme-sas",
    canonical_name: "Acme SAS",
    display_name: "Acme SAS",
    type: "organization",
    vertical: "m&a",
    roles: [],
    aliases: [],
    source_documents: [],
    mention_count: 1,
    vertical_metadata: {},
    first_seen: "2026-07-31T00:00:00.000Z",
    last_seen: "2026-07-31T00:00:00.000Z",
    ...overrides,
  };
}

describe("vaultRelativeLink (Track I2)", () => {
  it("links from a document up into entities/", () => {
    expect(vaultRelativeLink("documents/note.md", "entities/acme-sas.md")).toBe("../entities/acme-sas.md");
  });

  it("links from an entity page back down into documents/", () => {
    expect(vaultRelativeLink("entities/acme-sas.md", "documents/note.md")).toBe("../documents/note.md");
  });

  it("needs no prefix when linking from a vault-root file", () => {
    expect(vaultRelativeLink("GLOSSARY.md", "entities/acme-sas.md")).toBe("entities/acme-sas.md");
  });
});

describe("buildEntityMarkdown (Track I2)", () => {
  it("includes type, vertical, roles, and mention count", () => {
    const entity = registryEntity({
      roles: ["target"],
      mention_count: 3,
    });

    const markdown = buildEntityMarkdown(entity, new Map());

    expect(markdown).toContain("# Acme SAS");
    expect(markdown).toContain("**Type:** organization");
    expect(markdown).toContain("**Vertical:** m&a");
    expect(markdown).toContain("**Roles:** target");
    expect(markdown).toContain("**Mentions:** 3");
  });

  it("backlinks to every source document via a relative path", () => {
    const entity = registryEntity({
      source_documents: ["doc-001", "doc-002"],
    });
    const docPathByDocId = new Map([
      ["doc-001", "documents/note.md"],
      ["doc-002", "documents/contract.md"],
    ]);

    const markdown = buildEntityMarkdown(entity, docPathByDocId);

    expect(markdown).toContain("[note.md](../documents/note.md)");
    expect(markdown).toContain("[contract.md](../documents/contract.md)");
  });

  it("skips a source_documents entry with no known output path", () => {
    const entity = registryEntity({ source_documents: ["doc-missing"] });

    const markdown = buildEntityMarkdown(entity, new Map());

    expect(markdown).not.toContain("doc-missing");
  });
});

describe("buildGlossaryMarkdown (Track I2)", () => {
  it("links every entity into entities/, sorted by name", () => {
    const entities = [
      registryEntity({ slug: "beta-sarl", display_name: "Beta SARL" }),
      registryEntity({ slug: "acme-sas", display_name: "Acme SAS" }),
    ];

    const markdown = buildGlossaryMarkdown(entities);

    const acmeIndex = markdown.indexOf("Acme SAS");
    const betaIndex = markdown.indexOf("Beta SARL");
    expect(acmeIndex).toBeGreaterThan(-1);
    expect(betaIndex).toBeGreaterThan(acmeIndex);
    expect(markdown).toContain("[Acme SAS](entities/acme-sas.md)");
  });
});

describe("buildReadme (Track I2)", () => {
  it("mentions the document count and every top-level vault path", () => {
    const readme = buildReadme(3);

    expect(readme).toContain("3 processed");
    expect(readme).toContain("documents/");
    expect(readme).toContain("entities/");
    expect(readme).toContain("GLOSSARY.md");
  });
});
