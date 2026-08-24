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
import { looksLikePseudonymToken } from "./redaction-modes";

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
 * Task 5.3 (spec §8 step 5, §9 Q4): `GLOSSARY.md` used to list every entity, grouped by
 * type, with no size limit — a large batch produced a large file a filesystem-MCP reader
 * had to load in full just to find one row (spec §4's argument for why bundle shape
 * matters to an agent, not a human, reading it). This gates it to the top `topN`
 * entities by mention count, with the complete per-type listing moved to
 * `indexes/by-type/<type>.md` (`buildByTypeIndex` below).
 *
 * `50`, measured rather than assumed (plan §5.3: "measure N against a real corpus
 * rather than adopting the spec's placeholder 50 unexamined") — against a 300-entity
 * synthetic corpus with a Zipfian mention-count distribution (`mention_count[i] ≈
 * 500/(i+1)`, a slowly-decaying shape deliberately harder to cover than a steeper
 * power law): N=50 produces a 3.8 KB / 53-line file capturing 71.6% of total mentions;
 * N=100 roughly doubles both size and captures 82.4%; N=25 is barely smaller than N=50
 * (1.9 KB) but only captures 60.6%. No sharp knee exists in this shape — the choice is a
 * size/coverage tradeoff, not a threshold discovery — and 50 lands where the file is
 * still cheap to read in full while surfacing most of what a reader is likely to want,
 * with the by-type index as the answer for anything past that. Revisit if a real
 * exported corpus's distribution turns out steeper or flatter than this synthetic one.
 */
const DEFAULT_GLOSSARY_TOP_N = 50;

export function buildGlossaryIndex(
  entities: RegistryEntity[],
  topN: number = DEFAULT_GLOSSARY_TOP_N,
): string {
  if (entities.length === 0) {
    return "# Glossary\n\nNo entities were detected in this batch.\n";
  }
  const types = Array.from(new Set(entities.map((e) => e.type))).sort();
  const top = [...entities]
    .sort((a, b) => b.mention_count - a.mention_count)
    .slice(0, topN);

  let md =
    `# Glossary\n\nTop ${top.length} of ${entities.length} entities in this batch, ` +
    "by mention count. Open an entry for its full detail and backlinks into the " +
    "documents that mention it.\n\n";
  for (const e of top) {
    const typeLabel = e.type.charAt(0).toUpperCase() + e.type.slice(1);
    const verticalInfo =
      e.vertical && e.vertical !== "shared" ? ` — ${e.vertical}` : "";
    const docCount = e.source_documents.length;
    md += `- [${e.display_name}](entities/${entityFileName(e)}) \`${typeLabel}\`${verticalInfo}, mentioned ${e.mention_count} time${e.mention_count > 1 ? "s" : ""} across ${docCount} document${docCount === 1 ? "" : "s"}\n`;
  }
  if (entities.length > topN) {
    md += "\n## Full index by type\n\n";
    for (const type of types) {
      md += `- [indexes/by-type/${type}.md](indexes/by-type/${type}.md)\n`;
    }
  }
  return md;
}

/**
 * The complete, ungated listing `buildGlossaryIndex` used to be, now split one file per
 * type so a reader who needs the full picture for `organization` doesn't also load every
 * `person`/`email`/`date` entity to get it. `../../` in the entity link, not
 * `entityFileName`'s bare form: this file lives two directories below the zip root
 * (`indexes/by-type/<type>.md`), not one (`entities/<file>.md` is a sibling of
 * `indexes/`, not of `by-type/`).
 */
export function buildByTypeIndex(type: string, entities: RegistryEntity[]): string {
  const typeLabel = type.charAt(0).toUpperCase() + type.slice(1);
  const sorted = [...entities].sort((a, b) =>
    a.display_name.localeCompare(b.display_name),
  );
  let md = `# ${typeLabel} entities\n\nEvery \`${type}\` entity detected across this batch, ${sorted.length} total.\n\n`;
  for (const e of sorted) {
    const verticalInfo =
      e.vertical && e.vertical !== "shared" ? ` — ${e.vertical}` : "";
    const docCount = e.source_documents.length;
    md += `- [${e.display_name}](../../entities/${entityFileName(e)})${verticalInfo}, mentioned ${e.mention_count} time${e.mention_count > 1 ? "s" : ""} across ${docCount} document${docCount === 1 ? "" : "s"}\n`;
  }
  return md;
}

/**
 * Task 5.3: `_index/entities.jsonl` — one JSON object per line, so `grep <entity-id>
 * _index/entities.jsonl` returns a single complete, parseable record. Deliberately not
 * pretty-printed JSON like `entities-registry.json`: a `grep` against a multi-line
 * pretty-printed object returns one unparseable fragment of it, which is exactly the
 * problem `entities-registry.json` already has for programmatic lookup and this file
 * exists to not repeat (spec §4).
 */
export function buildEntitiesJsonl(entities: RegistryEntity[]): string {
  return entities
    .map((e) =>
      JSON.stringify({
        id: e.id,
        name: e.display_name,
        type: e.type,
        vertical: e.vertical,
        mention_count: e.mention_count,
        document_count: e.source_documents.length,
        aliases: e.aliases,
      }),
    )
    .join("\n");
}

/** `_index/documents.jsonl` — the document-side counterpart to `buildEntitiesJsonl`. */
export function buildDocumentsJsonl(
  results: ProcessedFile[],
  docPaths: Map<string, string>,
): string {
  const docIdByPath = new Map(
    Array.from(docPaths.entries()).map(([docId, path]) => [path, docId]),
  );
  return results
    .map((r) => {
      const path = "documents/" + r.name;
      return JSON.stringify({
        id: docIdByPath.get(path) ?? null,
        path,
        entity_count: r.entities.length,
        pii_entities_found: r.frontmatter.piiEntitiesFound,
      });
    })
    .join("\n");
}

/**
 * Task 5.3: `indexes/timeline.md` — date entities, chronological where the date could be
 * parsed unambiguously.
 *
 * Deliberately scoped to strict ISO `YYYY-MM-DD` only, not full bilingual date parsing.
 * This corpus's date entities can be French or English, numeric or spelled-out (see
 * `lib/ner-bridge.ts`'s `DATE_PATTERN` — ISO, `d/m/y` variants, and month names in both
 * languages all get detected as `date` entities), and correctly ordering "15 mars 2024"
 * against "03/15/2024" against "March 15, 2024" is a real parsing project of its own —
 * silently mis-sorting one format against another would be worse than not sorting it at
 * all. ISO strings sort correctly as plain strings (lexicographic order is chronological
 * order for `YYYY-MM-DD`), so no date-math library is needed for the part this does
 * handle. Everything else lands in an explicitly-labelled, alphabetically-sorted
 * "unparsed" section rather than being silently dropped or wrongly ordered.
 */
export function buildTimelineIndex(entities: RegistryEntity[]): string {
  const ISO_DATE = /^\d{4}-\d{2}-\d{2}$/;
  const dateEntities = entities.filter((e) => e.type === "date");
  if (dateEntities.length === 0) {
    return "# Timeline\n\nNo date entities were detected in this batch.\n";
  }

  const describe = (e: RegistryEntity, link: string) => {
    const docCount = e.source_documents.length;
    return `- [${e.display_name}](${link}) — mentioned ${e.mention_count} time${e.mention_count > 1 ? "s" : ""} across ${docCount} document${docCount === 1 ? "" : "s"}\n`;
  };

  const parseable = dateEntities
    .filter((e) => ISO_DATE.test(e.display_name.trim()))
    .sort((a, b) => a.display_name.localeCompare(b.display_name));
  const unparseable = dateEntities
    .filter((e) => !ISO_DATE.test(e.display_name.trim()))
    .sort((a, b) => a.display_name.localeCompare(b.display_name));

  let md =
    "# Timeline\n\nDate entities detected across this batch, in chronological order " +
    "where the date could be parsed unambiguously (strict ISO `YYYY-MM-DD` only).\n\n";
  for (const e of parseable) md += describe(e, `../entities/${entityFileName(e)}`);
  if (unparseable.length > 0) {
    md +=
      "\n## Other dates (format not recognised for chronological sorting, alphabetical)\n\n";
    for (const e of unparseable) md += describe(e, `../entities/${entityFileName(e)}`);
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

- **\`CLAUDE.md\` / \`AGENTS.md\`** — byte-identical routing instructions,
  auto-loaded by Claude and by Codex-family agents respectively. If you're
  reading this file *instead* of one of those, an AI session opening this
  bundle already has the short version.
- **\`documents/\`** — one markdown file per source document, at the same
  relative path it was uploaded from. Each has multi-line YAML frontmatter
  (source name, type, processing time, PII count, entity ids, a coarse
  \`doc_type\`) followed by the extracted content, with named entities linked
  to their file under \`entities/\`, a local \`## Entities\` summary, and — when
  this document shares entities with others in the batch — a
  \`## Related documents\` section naming which entities are shared.
- **\`entities/\`** — one file per distinct entity across the whole batch:
  type, vertical, roles, aliases, and a backlink to every document that
  mentions it. This is what makes the bundle RAG-ready rather than merely
  readable — open \`entities/organization-acme-sas.md\` and find every
  document naming Acme SAS, without reading every file in \`documents/\`.
- **\`GLOSSARY.md\`** — the top entities by mention count, not the full list;
  see \`indexes/by-type/\` for that. Start here for "what entities does this
  bundle know about".
- **\`indexes/by-type/<type>.md\`** — the complete, ungated listing for one
  entity type (every \`organization\`, every \`person\`, ...).
- **\`indexes/timeline.md\`** — date entities, chronological where the date
  parses unambiguously.
- **\`_index/entities.jsonl\`, \`_index/documents.jsonl\`** — one JSON record
  per line. Grep these for a specific id or path instead of loading
  \`entities-registry.json\` whole.
- **\`_manifest.json\`** — the file list for this batch, with per-file entity
  counts.
- **\`entities-registry.json\`** — every entity across the whole batch, with
  which document(s) it appears in and co-occurrence relationships between
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
documents), start from \`GLOSSARY.md\`, \`indexes/\`, \`_index/\`, \`entities/\`,
\`entities-registry.json\` or \`kg-export/\`, not by reading every file in
\`documents/\`. For a single document's content, its own \`.md\` file is
self-contained — frontmatter, prose, local entity summary, and related-document
pointers together.
`;
}

/**
 * Task 5.1 (spec §8 step 5, §4/§5.1.1): `README.md` is prose a session reads only if
 * something prompts it. `CLAUDE.md` and `AGENTS.md` are auto-loaded as instructions by
 * their respective runtimes (Claude Desktop/Code, and Codex-family agents) — this is the
 * routing table and redaction contract those runtimes actually receive.
 *
 * Written to *both* filenames from this one string (see `assembleZip`), not authored
 * twice: the redaction contract below is exactly the content that must not drift between
 * runtimes — a bundle that explains the pseudonym scheme to Claude but not to Codex is
 * one where a runtime that missed it silently treats `[PERSON:session:a41f]` as noise
 * instead of a stable identity.
 *
 * The redaction paragraph reflects what this batch *actually* produced, not just what
 * `config.redactionMode` says was configured — `pseudonymize` silently degrades to
 * mask-shaped output when no passphrase is given (`AppConfig.pseudonymPassphrase`'s doc
 * comment), and a bundle claiming "tokens are stable identities" in that case would be
 * false. `registry` is checked for at least one token-shaped entity name
 * (`looksLikePseudonymToken`) as the actual-effect signal, not `config` alone.
 */
function buildAgentInstructions(
  config: AppConfig,
  registry: BatchEntityRegistry,
): string {
  const routingTable = `## Answering questions from this bundle

| Question | Read first |
| --- | --- |
| Who or what is X | \`entities/<type>-<slug>.md\` |
| Which documents mention X | same file, its "Appears in" section |
| What entities does this bundle know about | \`GLOSSARY.md\`, then \`indexes/by-type/<type>.md\` for the full list |
| Documents about a topic or date | \`indexes/timeline.md\` for dates; \`indexes/by-type/\` otherwise |
| Documents related to the one you're reading | that document's own "## Related documents" section |
| Cross-document facts, relationships, lookups by id | \`_index/entities.jsonl\`, \`_index/documents.jsonl\` (one record per line — grep, don't load whole), or \`entities-registry.json\`/\`kg-export/\` for the full structure |

Do not read every file in \`documents/\` to answer a question that spans more than one
document — start from the entries above and follow the links out.`;

  let redactionParagraph: string;
  if (!config.redactPiiInOutput) {
    redactionParagraph = `PII detection ran in scan-only mode for this batch: nothing in
the exported documents was redacted. Any PII present in the source documents is present
here too.`;
  } else if (config.redactionMode === "pseudonymize") {
    const hasConfirmedTokens = registry
      .getEntities()
      .some((e) => looksLikePseudonymToken(e.display_name));
    redactionParagraph = hasConfirmedTokens
      ? `This bundle was processed in \`pseudonymize\` mode. Tokens of the form
\`[LABEL:key_id:...]\` (e.g. \`[PERSON:session:a41f7c2b9e3d]\`) are stable, non-identifying
identities: the same real-world entity always produces the same token everywhere in this
bundle, so treat a token as a consistent identity across every document that names it —
including the token-named files under \`entities/\`, which exist and carry full
backlinks for exactly this reason. You cannot resolve a token to a real name from
anything in this bundle, and should not attempt to.`
      : `This batch was configured for \`pseudonymize\` mode, but no entity in this bundle
carries a pseudonym token — either nothing in the source documents needed redacting, or
pseudonymization did not take effect (commonly: no passphrase was supplied, which
silently falls back to masking). Do not assume any name in this bundle is a stable token
unless it visibly matches \`[LABEL:key_id:...]\`.`;
  } else if (config.redactionMode === "hash") {
    redactionParagraph = `This bundle was processed in \`hash\` mode. Redacted spans are
replaced with a keyed digest (e.g. \`#email:1a2b3c4d5e6f7890\`); the same real value
always produces the same digest within this batch, so — like \`pseudonymize\` — two
identical digests denote the same underlying value, but the digest cannot be reversed to
recover it.`;
  } else if (config.redactionMode === "remove") {
    redactionParagraph = `This bundle was processed in \`remove\` mode. Redacted spans
were deleted outright, with nothing left in their place. There is no placeholder, token,
or digest to reason about for redacted content — treat a sentence with content missing
as missing, not as a value you could infer.`;
  } else {
    redactionParagraph = `This bundle was processed in \`mask\` mode. Redacted spans are
replaced with a fixed placeholder for their category (e.g. \`[EMAIL]\`) — every
redaction of the same category looks identical, so a placeholder does **not** identify
which specific value was removed, and two \`[EMAIL]\` placeholders in different places
are not necessarily the same email address.`;
  }

  return `# Agent instructions for this bundle

${routingTable}

## Redaction contract

${redactionParagraph}
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
  // Task 5.1: one string, two files — see `buildAgentInstructions`'s header for why this
  // must not be two independently-authored texts.
  const agentInstructions = buildAgentInstructions(config, registry);
  zip.file("CLAUDE.md", agentInstructions);
  zip.file("AGENTS.md", agentInstructions);

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

  // Task 5.3: the full per-type listings GLOSSARY.md's top-N gate now points readers to,
  // plus the greppable jsonl sidecars and the date-entity timeline.
  const byTypeFolder = zip.folder("indexes")?.folder("by-type");
  const entitiesByType = new Map<string, RegistryEntity[]>();
  for (const entity of registry.getEntities()) {
    const list = entitiesByType.get(entity.type) ?? [];
    list.push(entity);
    entitiesByType.set(entity.type, list);
  }
  for (const [type, typeEntities] of entitiesByType) {
    byTypeFolder?.file(`${type}.md`, buildByTypeIndex(type, typeEntities));
  }
  zip.file("indexes/timeline.md", buildTimelineIndex(registry.getEntities()));
  zip.file("_index/entities.jsonl", buildEntitiesJsonl(registry.getEntities()));
  zip.file("_index/documents.jsonl", buildDocumentsJsonl(results, docPaths));

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
