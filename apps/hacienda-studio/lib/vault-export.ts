import type { RegistryEntity } from "./registry";

/**
 * Track I2: the exported zip is a "vault" of relative markdown links (README.md,
 * documents/*.md, entities/<slug>.md, GLOSSARY.md), not a flat bundle — the goal is
 * that an agent (or a human in Obsidian/Claude Desktop) can navigate it purely by
 * following links, the way it would navigate any other local markdown vault. This
 * replaces the old `entity:type/slug` custom URI scheme, which no markdown renderer
 * or file browser could actually follow.
 *
 * All paths passed in/out of this module are vault-relative (POSIX `/`, no leading
 * `./` or `/`), e.g. `"documents/note.md"` or `"entities/acme-sas.md"`.
 */

/**
 * Computes the relative markdown link from one vault path to another, prefixing
 * `../` once per directory level `fromPath` is nested under. Both paths are file
 * paths (not directories), so only `fromPath`'s directory component contributes
 * `../` segments. Kept path-depth-aware (rather than hardcoding one `../`) so
 * nested document folders (Track I1, not yet implemented) don't silently break
 * every existing link.
 */
export function vaultRelativeLink(fromPath: string, toPath: string): string {
  const fromDepth = fromPath.split("/").length - 1;
  return "../".repeat(fromDepth) + toPath;
}

/**
 * One file per entity (`entities/<slug>.md`): type, vertical, roles, aliases, and
 * backlinks to every document it was mentioned in. `docPathByDocId` maps each
 * `docId` (e.g. `"doc-001"`) to its vault-relative output path (e.g.
 * `"documents/note.md"`) so backlinks can be computed relative to this file.
 */
export function buildEntityMarkdown(entity: RegistryEntity, docPathByDocId: Map<string, string>): string {
  const entityPath = `entities/${entity.slug}.md`;
  const lines: string[] = [];

  lines.push(`# ${entity.display_name}`);
  lines.push("");
  lines.push(`- **Type:** ${entity.type}`);
  lines.push(`- **Vertical:** ${entity.vertical}`);
  if (entity.sector) {
    lines.push(`- **Sector:** ${entity.sector}`);
  }
  if (entity.roles.length > 0) {
    lines.push(`- **Roles:** ${entity.roles.join(", ")}`);
  }
  if (entity.aliases.length > 0) {
    lines.push(`- **Aliases:** ${entity.aliases.join(", ")}`);
  }
  lines.push(`- **Mentions:** ${entity.mention_count}`);
  lines.push("");
  lines.push("## Mentioned In");
  lines.push("");
  for (const docId of entity.source_documents) {
    const docPath = docPathByDocId.get(docId);
    if (!docPath) continue;
    const docName = docPath.split("/").pop()!;
    lines.push(`- [${docName}](${vaultRelativeLink(entityPath, docPath)})`);
  }
  lines.push("");

  return lines.join("\n");
}

/**
 * `GLOSSARY.md`: an index of every entity, linking into `entities/`. Lives at the
 * vault root, so entity links need no `../` prefix.
 */
export function buildGlossaryMarkdown(entities: RegistryEntity[]): string {
  const lines: string[] = ["# Glossary", "", "| Entity | Type | Vertical | Mentions |", "| --- | --- | --- | --- |"];

  const sorted = [...entities].sort((a, b) => a.display_name.localeCompare(b.display_name));
  for (const entity of sorted) {
    lines.push(
      `| [${entity.display_name}](entities/${entity.slug}.md) | ${entity.type} | ${entity.vertical} | ${entity.mention_count} |`,
    );
  }
  lines.push("");

  return lines.join("\n");
}

/**
 * `README.md`: the vault's entry point, telling an agent what this bundle is and
 * how to navigate it. Static aside from the document count, since the structure
 * itself (not the content) is what needs explaining.
 */
export function buildReadme(documentCount: number): string {
  return `# Document Vault

This is an exported vault from Hacienda Studio, containing ${documentCount} processed
document(s) and the entities found within them. All links are relative markdown
links — follow them like any other local file.

## Structure

- \`documents/\` — one markdown file per input document, with entity mentions
  linked to their \`entities/\` page.
- \`entities/\` — one markdown file per entity (person, organization, etc.), with
  backlinks to every document it was mentioned in.
- \`GLOSSARY.md\` — an index of every entity, linking into \`entities/\`.
- \`_manifest.json\` — the list of processed documents and their entity counts.
- \`entities-registry.json\` — the full entity/relationship registry as structured data.
- \`kg-export/\` — the same registry exported as Neo4j Cypher, NetworkX JSON, and RDF Turtle.

## How to use this vault

Start at \`GLOSSARY.md\` to see every entity, or open a file under \`documents/\`
directly. Follow the links in either direction to move between a document and the
entities it mentions.
`;
}
