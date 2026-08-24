/**
 * Track K/Phase 2: split out of `worker/pipeline.ts` for the same reason `lib/annotate.ts`
 * was (see that file's header) — `assembleZip` needs to run from the worker's on-demand
 * `"build-zip"` message handler, not just once at the end of `processFiles()`, and
 * `worker/pipeline.ts` re-exports `buildEntityFile`/`buildGlossaryIndex` unchanged so
 * nothing importing them from `"./pipeline"` (the vitest suite included) needs to know
 * they moved.
 *
 * This replaces the zip-building block that used to run unconditionally at the end of
 * `processFiles()` and auto-download the result — building the zip is now a deliberate,
 * on-demand action (`App.tsx`'s "Download redacted zip" button on the file-browser
 * screen), so `assembleZip` takes the finished batch's state as plain data instead of
 * reading module-level variables.
 */
import JSZip from "jszip";
import type { ProcessedFile, AppConfig } from "./types";
import type { BatchEntityRegistry, RegistryEntity } from "./registry";
import { KGExporter } from "./kg-export";
import { entityFileName, relativeDocLink } from "./annotate";

export interface ZipBatch {
  results: ProcessedFile[];
  registry: BatchEntityRegistry;
  docPaths: Map<string, string>;
  config: AppConfig;
}

/**
 * Track I2: "one file per entity, with backlinks." `docLinks` are already
 * `documents/...` paths (see `processFiles`'s `docPaths` map) — sorted by
 * the caller so file output is deterministic across runs.
 */
export function buildEntityFile(
  entity: RegistryEntity,
  docLinks: string[],
): string {
  const typeLabel = entity.type.charAt(0).toUpperCase() + entity.type.slice(1);
  const lines = [`# ${entity.display_name}`, "", `- **Type:** ${typeLabel}`];
  if (entity.vertical) lines.push(`- **Vertical:** ${entity.vertical}`);
  if (entity.sector) lines.push(`- **Sector:** ${entity.sector}`);
  if (entity.roles.length) lines.push(`- **Roles:** ${entity.roles.join(", ")}`);
  if (entity.aliases.length)
    lines.push(`- **Aliases:** ${entity.aliases.join(", ")}`);
  lines.push(
    `- **Mentions:** ${entity.mention_count} across ${docLinks.length} document${docLinks.length === 1 ? "" : "s"}`,
  );
  lines.push("", "## Appears in", "");
  for (const docPath of docLinks) {
    lines.push(`- [${docPath.replace(/^documents\//, "")}](${relativeDocLink(docPath)})`);
  }
  return lines.join("\n") + "\n";
}

/**
 * Track I2: "GLOSSARY.md is the entry point" — the global index into
 * `entities/`, grouped by type and sorted for deterministic output.
 */
export function buildGlossaryIndex(entities: RegistryEntity[]): string {
  if (entities.length === 0) {
    return "# Glossary\n\nNo entities were detected in this batch.\n";
  }
  const byType = new Map<string, RegistryEntity[]>();
  for (const e of entities) {
    const list = byType.get(e.type) ?? [];
    list.push(e);
    byType.set(e.type, list);
  }
  let md =
    "# Glossary\n\nEvery entity detected across this batch. Open an entry " +
    "for its full detail and backlinks into the documents that mention it.\n";
  for (const type of Array.from(byType.keys()).sort()) {
    const typeLabel = type.charAt(0).toUpperCase() + type.slice(1);
    md += `\n## ${typeLabel}\n\n`;
    const sorted = byType
      .get(type)!
      .sort((a, b) => a.display_name.localeCompare(b.display_name));
    for (const e of sorted) {
      const verticalInfo =
        e.vertical && e.vertical !== "shared" ? ` — ${e.vertical}` : "";
      const docCount = e.source_documents.length;
      md += `- [${e.display_name}](entities/${entityFileName(e)})${verticalInfo}, mentioned ${e.mention_count} time${e.mention_count > 1 ? "s" : ""} across ${docCount} document${docCount === 1 ? "" : "s"}\n`;
    }
  }
  return md;
}

/**
 * Track G3: without this, a Claude Desktop session that opens the zip sees a
 * pile of markdown files, a JSON registry and a `kg-export/` folder with no
 * explanation — nothing tells it the registry and KG files exist to answer
 * cross-document questions the prose alone can't (which entities appear in
 * multiple files, how they relate), so a session would only ever read the
 * prose. `fileCount`/`entityCount` are computed by the caller, which already
 * has `results`/`registry` in scope; this function only formats.
 */
function buildBundleReadme(fileCount: number, entityCount: number): string {
  const documents = fileCount === 1 ? "document" : "documents";
  const entities = entityCount === 1 ? "entity" : "entities";
  return `# Hacienda Studio export

This bundle was produced entirely in-browser (Hacienda Studio) — no document
left the device it was processed on. It contains ${fileCount} processed
${documents} and ${entityCount} distinct ${entities} across them.

## What's in here

- **\`documents/\`** — one markdown file per source document, at the same
  relative path it was uploaded from. Each has YAML frontmatter (source name,
  type, processing time, PII count) followed by the extracted content, with
  named entities linked to their file under \`entities/\`, and a local
  \`## Entities\` summary at the bottom.
- **\`entities/\`** — one file per distinct entity across the whole batch:
  type, vertical, roles, aliases, and a backlink to every document that
  mentions it. This is what makes the bundle RAG-ready rather than merely
  readable — open \`entities/organization-acme-sas.md\` and find every
  document naming Acme SAS, without reading every file in \`documents/\`.
- **\`GLOSSARY.md\`** — the index into \`entities/\`, grouped by type. Start
  here for "what entities does this bundle know about".
- **\`_manifest.json\`** — the file list for this batch, with per-file entity
  counts.
- **\`entities-registry.json\`** — every entity across the whole batch, with
  which document(s) it appears in and inferred relationships between
  entities. Use this, not just the prose, to answer questions that span more
  than one document — an entity mentioned in three files only has one row
  here, not three.
- **\`kg-export/\`** — the same registry as a knowledge graph, in three
  formats: \`neo4j.cypher\` (importable into Neo4j), \`networkx.json\`
  (Python's NetworkX), and \`rdf.ttl\` (RDF/Turtle).

## What the graph edges mean

Every edge is \`co_occurs_with\`, and it means exactly one thing: **the two
entities were named close together in the same document** — same sentence
(\`confidence: 0.6\`) or same paragraph (\`confidence: 0.3\`). \`confidence\` is
a proximity strength, not a probability.

Co-occurrence is **not** a relationship. If a person and a company are named
in the same sentence, this bundle records that they were named together and
nothing more — not that the person works there, owns it, or has ever heard of
it. Any such conclusion has to come from reading the passage, which is what
the edge is for: it tells you *where to look*, not what you will find.

Pairs farther apart than a paragraph get no edge. That both entities appear
somewhere in the same document is already in \`document_entities\` in
\`entities-registry.json\`; it is not repeated here as a relationship.

## Reading this bundle

For cross-document questions (shared entities, relationships between
documents), start from \`GLOSSARY.md\`, \`entities/\`, \`entities-registry.json\`
or \`kg-export/\`, not by reading every file in \`documents/\`. For a single
document's content, its own \`.md\` file is self-contained — frontmatter,
prose, and local entity summary together.
`;
}

/**
 * `overrides` keys are `ProcessedFile.name` (the `.md` output name) — when present, its
 * value replaces `result.markdown` in the exported `documents/*.md` entry. This is how
 * Phase 3's split-view redaction edits (K2) and Track I4's findings edits reach the zip
 * without the worker needing to know about either: `App.tsx`'s `resolveExportContent`
 * computes the override, this function just applies it. Every other zip member
 * (registry, KG export, entity files, glossary) is derived from `registry`/`docPaths` as
 * they were at processing time — Track A2's export-time filtering already happened in
 * `worker/pipeline.ts`'s `processFile`, so no override propagates into those.
 */
export async function assembleZip(
  batch: ZipBatch,
  overrides: Record<string, string> = {},
): Promise<Blob> {
  const { results, registry, docPaths, config } = batch;
  const zip = new JSZip();
  for (const r of results) {
    zip.file("documents/" + r.name, overrides[r.name] ?? r.markdown);
  }
  const manifest = {
    files: results.map((r) => ({
      name: r.name,
      entityCount: r.entities.length,
    })),
    generated: new Date().toISOString(),
    // Task 1 (spec §8 step 2): before this, every relationship in `entities-registry.json`
    // and `kg-export/` could be a fabricated `works_for`/`partner_of`/`contact_email` claim
    // asserted from bare co-occurrence. From this version on, every relationship type is
    // `co_occurs_with`, scored by proximity, and asserts nothing beyond "named near each
    // other". A reader (human or agent) diffing two bundles needs this to tell which
    // semantics produced a given `entities-registry.json` without inspecting every edge —
    // silently treating an old bundle's `works_for` edges as this version's weaker claim
    // would be as wrong as the bug this field exists to flag.
    relationshipSemanticsVersion: 2,
  };
  zip.file("_manifest.json", JSON.stringify(manifest, null, 2));

  const registryJson = {
    ...registry.toJSON(),
    ...(config.enableTranscription && {
      transcription: {
        model: config.transcriptionModel,
        language: config.transcriptionLanguage,
        enabled: true,
      },
    }),
  };
  zip.file("entities-registry.json", JSON.stringify(registryJson, null, 2));
  zip.file(
    "README.md",
    buildBundleReadme(
      results.length,
      registryJson.entity_registry.entities.length,
    ),
  );

  const entitiesFolder = zip.folder("entities");
  for (const entity of registry.getEntities()) {
    const docLinks = entity.source_documents
      .map((docId) => docPaths.get(docId))
      .filter((p): p is string => !!p)
      .sort();
    entitiesFolder?.file(
      entityFileName(entity),
      buildEntityFile(entity, docLinks),
    );
  }
  zip.file("GLOSSARY.md", buildGlossaryIndex(registry.getEntities()));

  const kgExporter = new KGExporter(registry);
  const kgFolder = zip.folder("kg-export");
  kgFolder?.file("neo4j.cypher", kgExporter.toCypher());
  kgFolder?.file(
    "networkx.json",
    JSON.stringify(kgExporter.toNetworkX(), null, 2),
  );
  kgFolder?.file("rdf.ttl", kgExporter.toRDF());

  return zip.generateAsync({ type: "blob" });
}
