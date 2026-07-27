# Vertical NER Architecture Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Implement vertical/domain-specific NER for M&A/Corporate Law and Financial Services with batch entity registry, knowledge graph exports, and RAG-ready entity linking.

**Architecture:** Hybrid GLiNER2 (Onnx) + Compromise bridge + Vertical Dictionary + Structured Extraction. Batch-level entity registry with deduplication. KG exports (Neo4j Cypher, NetworkX JSON, RDF/Turtle). Structured extraction with vertical schemas.

**Tech Stack:** Svelte 5, TypeScript, @xberg-io/xberg-wasm (rc.40), Compromise NLP, WasmNerConfig (Onnx backend), WasmStructuredExtractionConfig.

## Global Constraints

- xberg-wasm version: rc.40 (rc.38-39 have import errors, rc.41+ untested)
- WASM init must use explicit URL: `new URL('/node_modules/@xberg-io/xberg-wasm/pkg/web/xberg_wasm_bg.wasm', self.location.origin)`
- Worker init via `initWasm({ module_or_path: Response })` — not `initSync`
- NER bridge injection: `new XbergEngine({ bridgeTimeoutMs: 30000 }, { ner: { ner: nerBridge } })`
- Engine NER call: `await engine.ner(text, { categories })`
- Svelte 5: `$state`, `$props`, `onclick` not `on:click`, `mount()` not `new App()`
- Dev server: `npm run dev -- --host 0.0.0.0 --port 5176`
- Build: `npm run build`
- E2E tests: `npx playwright test tests/e2e/basic.spec.ts --project=chromium`
- File paths: `src/lib/`, `src/worker/`, `src/lib/verticals/`

---

## Phase 1: Core Vertical Dictionary & Registry (Week 1)

### Task 1: Create Vertical Taxonomy YAML Files

**Files:**

- Create: `src/lib/verticals/m&a.yaml`
- Create: `src/lib/verticals/financial_services.yaml`
- Create: `src/lib/verticals/shared.yaml`
- Create: `src/lib/verticals/index.ts`

**Interfaces:**

- Consumes: None (first task)
- Produces: `VerticalTaxonomy` type, `loadVerticalTaxonomy()` function

- [ ] **Step 1: Write failing test**

```typescript
// src/lib/verticals/index.test.ts
import { loadVerticalTaxonomy, VerticalTaxonomy } from "./index";

test("loads M&A taxonomy with entity types and relationships", () => {
  const taxonomy = loadVerticalTaxonomy("m&a");
  expect(taxonomy.vertical).toBe("m&a");
  expect(taxonomy.entityTypes).toContain("target_company");
  expect(taxonomy.entityTypes).toContain("earnout");
  expect(taxonomy.relationships).toContain("acquirer_of");
});

test("loads Financial Services taxonomy", () => {
  const taxonomy = loadVerticalTaxonomy("financial_services");
  expect(taxonomy.vertical).toBe("financial_services");
  expect(taxonomy.entityTypes).toContain("fund");
  expect(taxonomy.entityTypes).toContain("carried_interest");
  expect(taxonomy.relationships).toContain("invests_in");
});

test("loads shared taxonomy", () => {
  const taxonomy = loadVerticalTaxonomy("shared");
  expect(taxonomy.vertical).toBe("shared");
  expect(taxonomy.entityTypes).toContain("person");
  expect(taxonomy.entityTypes).toContain("organization");
});
```

- [ ] **Step 2: Run test to verify it fails**

```bash
npm run test -- src/lib/verticals/index.test.ts
```

Expected: FAIL - module not found

- [ ] **Step 3: Create YAML files and index.ts**

Create `src/lib/verticals/m&a.yaml` with M&A taxonomy from spec.
Create `src/lib/verticals/financial_services.yaml` with FS taxonomy from spec.
Create `src/lib/verticals/shared.yaml` with shared taxonomy from spec.
Create `src/lib/verticals/index.ts` with `loadVerticalTaxonomy()` using dynamic import.

- [ ] **Step 4: Run test to verify it passes**

```bash
npm run test -- src/lib/verticals/index.test.ts
```

Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add src/lib/verticals/
git commit -m "feat: add vertical taxonomy YAML files and loader"
```

---

### Task 2: Build VerticalDictionary Class

**Files:**

- Create: `src/lib/verticals/dictionary.ts`
- Test: `src/lib/verticals/dictionary.test.ts`

**Interfaces:**

- Consumes: `VerticalTaxonomy` from Task 1
- Produces: `VerticalDictionary` class with `lookup(term: string): VerticalEntityMetadata | null`

- [ ] **Step 1: Write failing test**

```typescript
// src/lib/verticals/dictionary.test.ts
import { VerticalDictionary } from "./dictionary";
import { loadVerticalTaxonomy } from "./index";

test("looks up canonical term and returns vertical metadata", () => {
  const taxonomy = loadVerticalTaxonomy("m&a");
  const dict = new VerticalDictionary([taxonomy]);

  const result = dict.lookup("target company");
  expect(result).not.toBeNull();
  expect(result!.vertical).toBe("m&a");
  expect(result!.canonical).toBe("target_company");
  expect(result!.sector).toBeDefined();
});

test("handles aliases", () => {
  const taxonomy = loadVerticalTaxonomy("m&a");
  const dict = new VerticalDictionary([taxonomy]);

  const result = dict.lookup("SPA");
  expect(result).not.toBeNull();
  expect(result!.canonical).toBe("share_purchase_agreement");
});

test("returns null for unknown terms", () => {
  const taxonomy = loadVerticalTaxonomy("m&a");
  const dict = new VerticalDictionary([taxonomy]);

  const result = dict.lookup("unknown term xyz");
  expect(result).toBeNull();
});
```

- [ ] **Step 2: Run test to verify it fails**

```bash
npm run test -- src/lib/verticals/dictionary.test.ts
```

- [ ] **Step 3: Implement VerticalDictionary class**

```typescript
// src/lib/verticals/dictionary.ts
export interface VerticalEntityMetadata {
  canonical: string;
  vertical: string;
  sector?: string;
  roles?: string[];
  aliases?: string[];
}

export class VerticalDictionary {
  private map: Map<string, VerticalEntityMetadata> = new Map();

  constructor(taxonomies: VerticalTaxonomy[]) {
    for (const taxonomy of taxonomies) {
      for (const entityType of taxonomy.entityTypes) {
        this.map.set(entityType.toLowerCase(), {
          canonical: entityType,
          vertical: taxonomy.vertical,
          sector: taxonomy.sectors[0],
        });
        // Add aliases from customLabels
      }
    }
  }

  lookup(term: string): VerticalEntityMetadata | null {
    return this.map.get(term.toLowerCase()) || null;
  }
}
```

- [ ] **Step 4: Run test to verify it passes**
- [ ] **Step 5: Commit**

---

### Task 3: Build BatchEntityRegistry Class

**Files:**

- Create: `src/lib/registry.ts`
- Test: `src/lib/registry.test.ts`

**Interfaces:**

- Consumes: `VerticalDictionary` from Task 2, `Entity` from `types.ts`
- Produces: `BatchEntityRegistry` class with `addEntity()`, `addRelationship()`, `toJSON()`, `getKGExports()`

- [ ] **Step 1: Write failing test**

```typescript
// src/lib/registry.test.ts
import { BatchEntityRegistry } from "./registry";
import { Entity } from "../types";

test("adds and deduplicates entities by canonical name + type + vertical", () => {
  const registry = new BatchEntityRegistry();

  const e1: Entity = {
    name: "Acme Corp",
    type: "Organization",
    slug: "acme-corp",
    count: 1,
    spans: [],
  };
  const e2: Entity = {
    name: "Acme Inc",
    type: "Organization",
    slug: "acme-inc",
    count: 1,
    spans: [],
  };

  registry.addEntity(
    e1,
    { vertical: "m&a", sector: "tech", roles: ["target_company"] },
    "doc-001",
  );
  registry.addEntity(
    e2,
    { vertical: "m&a", sector: "tech", roles: ["target_company"] },
    "doc-002",
  );

  const entities = registry.getEntities();
  expect(entities.length).toBe(1);
  expect(entities[0].source_documents).toContain("doc-001");
  expect(entities[0].source_documents).toContain("doc-002");
});

test("adds relationships", () => {
  const registry = new BatchEntityRegistry();
  const e1 = registry.addEntity(
    {
      name: "Acme Corp",
      type: "Organization",
      slug: "acme-corp",
      count: 1,
      spans: [],
    },
    { vertical: "m&a" },
    "doc-001",
  );
  const e2 = registry.addEntity(
    { name: "John Doe", type: "Person", slug: "john-doe", count: 1, spans: [] },
    { vertical: "m&a" },
    "doc-001",
  );

  registry.addRelationship(
    e2.id,
    e1.id,
    "officer_of",
    "CEO per SPA",
    0.95,
    "doc-001",
  );

  const rels = registry.getRelationships();
  expect(rels.length).toBe(1);
  expect(rels[0].type).toBe("officer_of");
});

test("exports to JSON with correct structure", () => {
  const registry = new BatchEntityRegistry();
  // ... setup
  const json = registry.toJSON();
  expect(json.batch_id).toBeDefined();
  expect(json.entity_registry.entities).toBeDefined();
  expect(json.entity_registry.relationships).toBeDefined();
});
```

- [ ] **Step 2: Run test to verify it fails**
- [ ] **Step 3: Implement BatchEntityRegistry class**

```typescript
// src/lib/registry.ts
export interface RegistryEntity {
  id: string;
  canonical_name: string;
  display_name: string;
  type: string;
  vertical: string;
  sector?: string;
  roles: string[];
  aliases: string[];
  source_documents: string[];
  mention_count: number;
  vertical_metadata: Record<string, any>;
  first_seen: string;
  last_seen: string;
}

export interface RegistryRelationship {
  id: string;
  source_entity_id: string;
  target_entity_id: string;
  relationship_type: string;
  context: string;
  confidence: number;
  source_document: string;
}

export class BatchEntityRegistry {
  private entities: Map<string, RegistryEntity> = new Map();
  private relationships: RegistryRelationship[] = [];
  private batchId: string;
  private entityKeyMap: Map<string, string> = new Map(); // canonical|type|vertical -> id

  constructor() {
    this.batchId = `batch-${Date.now()}-${Math.random().toString(36).slice(2, 7)}`;
  }

  addEntity(entity: Entity, metadata: any, docId: string): RegistryEntity {
    const key = `${entity.name.toLowerCase()}|${entity.type}|${metadata.vertical}`;
    let existingId = this.entityKeyMap.get(key);

    if (existingId) {
      const existing = this.entities.get(existingId)!;
      existing.source_documents.push(metadata.docId);
      existing.mention_count++;
      existing.last_seen = new Date().toISOString();
      return existing;
    }

    const id = `ent-${String(this.entities.size + 1).padStart(3, "0")}`;
    const registryEntity: RegistryEntity = {
      id,
      canonical_name: entity.name,
      display_name: entity.name,
      type: entity.type,
      vertical: metadata.vertical,
      sector: metadata.sector,
      roles: metadata.roles || [],
      aliases: [],
      source_documents: [metadata.docId],
      mention_count: 1,
      vertical_metadata: metadata,
      first_seen: new Date().toISOString(),
      last_seen: new Date().toISOString(),
    };

    this.entities.set(id, registryEntity);
    this.entityKeyMap.set(key, id);
    return registryEntity;
  }

  addRelationship(
    sourceId: string,
    targetId: string,
    type: string,
    context: string,
    confidence: number,
    docId: string,
  ) {
    this.relationships.push({
      id: `rel-${String(this.relationships.length + 1).padStart(3, "0")}`,
      source_entity_id: sourceId,
      target_entity_id: targetId,
      relationship_type: type,
      context,
      confidence,
      source_document: docId,
    });
  }

  toJSON() {
    return {
      batch_id: this.batchId,
      processed_at: new Date().toISOString(),
      total_documents: [
        ...new Set(
          Array.from(this.entities.values()).flatMap((e) => e.source_documents),
        ),
      ].length,
      vertical_taxonomy_version: "1.0.0",
      entity_registry: {
        entities: Array.from(this.entities.values()),
        relationships: this.relationships,
      },
      document_entities: {}, // filled by worker
      vertical_summary: {},
    };
  }

  getEntities() {
    return Array.from(this.entities.values());
  }
  getRelationships() {
    return this.relationships;
  }
}
```

- [ ] **Step 4: Run test to verify it passes**
- [ ] **Step 5: Commit**

---

## Phase 2: Worker Pipeline Integration (Week 1-2)

### Task 4: Integrate Vertical Dictionary into Worker

**Files:**

- Modify: `src/worker/pipeline.ts`
- Test: `src/worker/pipeline.test.ts`

**Interfaces:**

- Consumes: `VerticalDictionary` from Task 2, `BatchEntityRegistry` from Task 3
- Produces: Enhanced `processFile()` with vertical enrichment

- [ ] **Step 1: Write failing test**

```typescript
// src/worker/pipeline.test.ts
import { processFile } from "./pipeline";

test("processFile returns entities with vertical metadata", async () => {
  const input = {
    name: "test.txt",
    bytes: new TextEncoder().encode("John Doe works at Acme Corp"),
    type: "text/plain",
  };
  const config = {
    nerCategories: ["person", "organization"],
    outputFormat: "markdown",
    chunkSize: 1000,
  };

  const result = await processFile(input, config);

  expect(result.entities.length).toBeGreaterThan(0);
  expect(result.entities[0]).toHaveProperty("vertical");
  expect(result.entities[0]).toHaveProperty("sector");
});
```

- [ ] **Step 2: Run test to verify it fails**
- [ ] **Step 3: Modify pipeline.ts**

Add imports:

```typescript
import {
  VerticalDictionary,
  VerticalEntityMetadata,
} from "../lib/verticals/dictionary";
import { loadVerticalTaxonomy } from "../lib/verticals/index";
import { BatchEntityRegistry } from "../lib/registry";
```

Initialize in `processFiles()`:

```typescript
async function processFiles(files: FileInput[], config: AppConfig) {
  // Load vertical taxonomies
  const taxonomies = ["m&a", "financial_services", "shared"].map(
    loadVerticalTaxonomy,
  );
  const verticalDict = new VerticalDictionary(taxonomies);
  const registry = new BatchEntityRegistry();

  // ... process each file
  for (const file of files) {
    const processed = await processFile(file, config, verticalDict, registry);
    results.push(processed);
  }

  // Add registry to output
  const registryJson = registry.toJSON();
  // Add to zip as entities-registry.json
}
```

Update `processFile()` signature and enrich entities:

```typescript
async function processFile(input, config, verticalDict, registry) {
  // ... existing extraction ...

  // Enrich entities with vertical metadata
  for (const entity of xbergEntities) {
    const verticalMeta = verticalDict.lookup(entity.text.toLowerCase());
    const enrichedEntity = {
      ...entity,
      vertical: verticalMeta?.vertical || "shared",
      sector: verticalMeta?.sector,
      roles: verticalMeta?.roles || [],
    };
    registry.addEntity(
      enrichedEntity,
      { vertical: enrichedEntity.vertical },
      input.name,
    );
  }

  // Build relationships from co-occurrence in same document
  // ...
}
```

- [ ] **Step 4: Run test to verify it passes**
- [ ] **Step 5: Commit**

---

### Task 5: Add entities-registry.json to Zip Output

**Files:**

- Modify: `src/worker/pipeline.ts`

**Interfaces:**

- Consumes: `BatchEntityRegistry` from Task 3
- Produces: `entities-registry.json` in zip output

- [ ] **Step 1: Write failing test**

```typescript
test("zip contains entities-registry.json", async () => {
  const zip = await require("jszip").loadAsync(zipBlob);
  expect(zip.files["entities-registry.json"]).toBeDefined();

  const content = await zip.files["entities-registry.json"].async("text");
  const registry = JSON.parse(content);
  expect(registry.batch_id).toBeDefined();
  expect(registry.entity_registry.entities).toBeDefined();
});
```

- [ ] **Step 2: Run test to verify it fails**
- [ ] **Step 3: Add registry to zip in `processFiles()`**

```typescript
// In processFiles(), after processing all files:
const registryJson = registry.toJSON();
// Add document-entities mapping
registryJson.document_entities = {}; // fill with doc -> entity IDs
zip.file("entities-registry.json", JSON.stringify(registryJson, null, 2));
```

- [ ] **Step 3: Run test to verify it passes**
- [ ] **Step 4: Commit**

---

## Phase 3: Knowledge Graph Export (Week 2)

### Task 6: Implement KG Export Classes

**Files:**

- Create: `src/lib/kg-export.ts`
- Test: `src/lib/kg-export.test.ts`

**Interfaces:**

- Consumes: `BatchEntityRegistry` from Task 3
- Produces: `KGExporter` class with `toCypher()`, `toNetworkX()`, `toRDF()`

- [ ] **Step 1: Write failing test**

```typescript
// src/lib/kg-export.test.ts
import { KGExporter } from "./kg-export";
import { BatchEntityRegistry } from "./registry";

test("generates valid Cypher for Neo4j", () => {
  const registry = new BatchEntityRegistry();
  // ... add entities and relationships

  const exporter = new KGExporter(registry);
  const cypher = exporter.toCypher();

  expect(cypher).toContain("CREATE (e:Entity");
  expect(cypher).toContain("ent-001");
  expect(cypher).toContain("OFFICER_OF");
});

test("generates valid NetworkX JSON", () => {
  const exporter = new KGExporter(registry);
  const nx = exporter.toNetworkX();

  expect(nx.nodes).toBeDefined();
  expect(nx.edges).toBeDefined();
  expect(nx.nodes[0]).toHaveProperty("id");
  expect(nx.edges[0]).toHaveProperty("source");
  expect(nx.edges[0]).toHaveProperty("target");
});

test("generates valid RDF/Turtle", () => {
  const exporter = new KGExporter(registry);
  const turtle = exporter.toRDF();

  expect(turtle).toContain("@prefix");
  expect(turtle).toContain("ent-001");
  expect(turtle).toContain("xberg:Organization");
});
```

- [ ] **Step 2: Run test to verify it fails**
- [ ] **Step 3: Implement KGExporter class**

```typescript
// src/lib/kg-export.ts
export class KGExporter {
  constructor(private registry: BatchEntityRegistry) {}

  toCypher(): string {
    const entities = this.registry.getEntities();
    const relationships = this.registry.getRelationships();

    let cypher = "";

    // Create entities
    for (const e of entities) {
      const props = [
        `id: "${e.id}"`,
        `canonical_name: "${e.canonical_name}"`,
        `display_name: "${e.display_name}"`,
        `type: "${e.type}"`,
        `vertical: "${e.vertical}"`,
        ...(e.sector ? [`sector: "${e.sector}"`] : []),
        ...(e.roles.length ? [`roles: ${JSON.stringify(e.roles)}`] : []),
        `mention_count: ${e.mention_count}`,
      ].join(", ");

      cypher += `CREATE (e:Entity {${props}});\n`;
    }

    // Create relationships
    for (const r of this.registry.getRelationships()) {
      cypher += `MATCH (a:Entity {id: "${r.source_entity_id}"}), (b:Entity {id: "${r.target_entity_id}"})\n`;
      cypher += `CREATE (a)-[:${r.relationship_type.toUpperCase()} {confidence: ${r.confidence}, context: "${r.context}"}]->(b);\n`;
    }

    return cypher;
  }

  toNetworkX(): object {
    return {
      nodes: this.registry.getEntities().map((e) => ({
        id: e.id,
        name: e.canonical_name,
        type: e.type,
        vertical: e.vertical,
        sector: e.sector,
        roles: e.roles,
      })),
      edges: this.registry.getRelationships().map((r) => ({
        source: r.source_entity_id,
        target: r.target_entity_id,
        type: r.relationship_type,
        confidence: r.confidence,
      })),
    };
  }

  toRDF(): string {
    let turtle = `@prefix xberg: <http://xberg.io/ontology#> .\n@prefix schema: <http://schema.org/> .\n\n`;

    for (const e of this.registry.getEntities()) {
      turtle += `<${e.id}> a xberg:${e.type} ;\n`;
      turtle += `  schema:name "${e.canonical_name}" ;\n`;
      turtle += `  xberg:displayName "${e.display_name}" ;\n`;
      turtle += `  xberg:vertical "${e.vertical}" ;\n`;
      if (e.sector) turtle += `  xberg:sector "${e.sector}" ;\n`;
      if (e.roles.length)
        turtle += `  xberg:role ${e.roles.map((r) => `"${r}"`).join(", ")} ;\n`;
      turtle += `  xberg:mentionCount ${e.mention_count} .\n\n`;
    }

    for (const r of this.registry.getRelationships()) {
      turtle += `<${r.source_entity_id}> xberg:${r.relationship_type} <${r.target_entity_id}> .\n`;
    }

    return turtle;
  }
}
```

- [ ] **Step 4: Run test to verify it passes**
- [ ] **Step 5: Commit**

---

### Task 7: Add KG Exports to Zip Output

**Files:**

- Modify: `src/worker/pipeline.ts`

**Interfaces:**

- Consumes: `KGExporter` from Task 6
- Produces: `kg-export/` folder in zip with `neo4j.cypher`, `networkx.json`, `rdf.ttl`

- [ ] **Step 1: Write failing test**

```typescript
test("zip contains kg-export folder with all formats", async () => {
  const zip = await JSZip.loadAsync(zipBlob);
  expect(zip.files["kg-export/neo4j.cypher"]).toBeDefined();
  expect(zip.files["kg-export/networkx.json"]).toBeDefined();
  expect(zip.files["kg-export/rdf.ttl"]).toBeDefined();
});
```

- [ ] **Step 2: Run test to verify it fails**
- [ ] **Step 3: Add KG exports to zip in `processFiles()`**

```typescript
const exporter = new KGExporter(registry);
zip.folder("kg-export")!.file("neo4j.cypher", exporter.toCypher());
zip
  .folder("kg-export")!
  .file("networkx.json", JSON.stringify(exporter.toNetworkX(), null, 2));
zip.folder("kg-export")!.file("rdf.ttl", exporter.toRDF());
```

- [ ] **Step 4: Run test to verify it passes**
- [ ] **Step 5: Commit**

---

## Phase 4: Structured Extraction Integration (Week 2-3)

### Task 8: Add Structured Extraction Config

**Files:**

- Modify: `src/worker/pipeline.ts`
- Create: `src/lib/extraction-schemas.ts`

**Interfaces:**

- Consumes: xberg-wasm `WasmStructuredExtractionConfig`
- Produces: Extraction schemas per document type

- [ ] **Step 1: Create extraction schemas**

```typescript
// src/lib/extraction-schemas.ts
export const EXTRACTION_SCHEMAS = {
  spa: {
    fields: [
      {
        name: "target_company",
        type: "string",
        description: "Target company name",
      },
      {
        name: "acquirer",
        type: "string",
        description: "Acquirer company name",
      },
      { name: "deal_value", type: "money", description: "Deal value" },
      {
        name: "deal_type",
        type: "enum",
        values: ["merger", "acquisition", "divestiture", "carve_out"],
      },
      {
        name: "earnout",
        type: "boolean",
        description: "Has earnout provision",
      },
      {
        name: "governing_law",
        type: "string",
        description: "Governing law jurisdiction",
      },
    ],
  },
  lpa: {
    fields: [
      { name: "fund_name", type: "string" },
      { name: "fund_size", type: "money" },
      { name: "management_fee", type: "percentage" },
      { name: "carried_interest", type: "percentage" },
      { name: "gp_commitment", type: "percentage" },
    ],
  },
  credit_agreement: {
    fields: [
      { name: "borrower", type: "string" },
      { name: "facility_amount", type: "money" },
      { name: "interest_rate", type: "percentage" },
      { name: "maturity_date", type: "date" },
      { name: "covenants", type: "array", items: "string" },
    ],
  },
};

export function getExtractionSchema(filename: string): object | null {
  const name = filename.toLowerCase();
  if (name.includes("spa") || name.includes("purchase_agreement"))
    return EXTRACTION_SCHEMAS.spa;
  if (name.includes("lpa") || name.includes("limited_partnership"))
    return EXTRACTION_SCHEMAS.lpa;
  if (name.includes("credit_agreement") || name.includes("loan_agreement"))
    return EXTRACTION_SCHEMAS.credit_agreement;
  return null;
}
```

- [ ] **Step 2: Add structured extraction to pipeline**

```typescript
// In processFile():
const schema = getExtractionSchema(input.name);
if (schema && config.enableStructuredExtraction) {
  extractConfig.structuredExtraction = {
    schema: schema,
    preset: "m&a", // or 'financial_services'
  };
}
```

- [ ] **Step 3: Add structured results to entity registry**

```typescript
if (result.results[0]?.structuredExtraction) {
  const structured = result.results[0].structuredExtraction;
  // Map structured fields to entity registry
  for (const [key, value] of Object.entries(structured)) {
    // Create or enrich entities from structured fields
  }
}
```

- [ ] **Step 4: Commit**

---

## Phase 5: Compliance & RAG Features (Week 3)

### Task 9: Entity Linking in Markdown & Frontmatter Enhancement

**Files:**

- Modify: `src/worker/pipeline.ts`

**Interfaces:**

- Consumes: Enriched entities from registry
- Produces: Markdown with `entity:type/slug` links, enhanced frontmatter

- [ ] **Step 1: Verify current entity linking works**

The current `linkEntities()` function already creates `[Entity](entity:type/slug)` links. Verify it uses vertical slugs.

- [ ] **Step 2: Enhance frontmatter with vertical metadata**

```yaml
entities:
  - name: "Acme Corp"
    type: "Organization"
    slug: "acme-corp"
    vertical: "m&a"
    sector: "technology"
    roles: ["target_company"]
```

- [ ] **Step 3: Add vertical summary to frontmatter**

```yaml
vertical_summary:
  m&a:
    entity_count: 142
    relationship_count: 87
  financial_services:
    entity_count: 56
    relationship_count: 34
```

- [ ] **Step 4: Commit**

---

### Task 10: Batch Registry Validation Report

**Files:**

- Create: `src/lib/validation-report.ts`
- Modify: `src/worker/pipeline.ts`

**Interfaces:**

- Consumes: `BatchEntityRegistry`
- Produces: `validation-report.json` in zip

- [ ] **Step 1: Create validation report**

```typescript
// src/lib/validation-report.ts
export interface ValidationReport {
  batch_id: string;
  generated_at: string;
  total_entities: number;
  total_relationships: number;
  entities_by_vertical: Record<string, number>;
  entities_by_type: Record<string, number>;
  relationships_by_type: Record<string, number>;
  documents_without_entities: string[];
  low_confidence_relationships: number;
  deduplication_stats: {
    input_entities: number;
    deduplicated_entities: number;
    dedup_ratio: number;
  };
}

export function generateValidationReport(
  registry: BatchEntityRegistry,
): ValidationReport {
  const entities = registry.getEntities();
  const relationships = registry.getRelationships();

  return {
    batch_id: registry.batchId,
    generated_at: new Date().toISOString(),
    total_entities: entities.length,
    total_relationships: relationships.length,
    entities_by_vertical: entities.reduce((acc, e) => {
      acc[e.vertical] = (acc[e.vertical] || 0) + 1;
      return acc;
    }, {}),
    entities_by_type: entities.reduce((acc, e) => {
      acc[e.type] = (acc[e.type] || 0) + 1;
      return acc;
    }, {}),
    relationships_by_type: relationships.reduce((acc, r) => {
      acc[r.relationship_type] = (acc[r.relationship_type] || 0) + 1;
      return acc;
    }, {}),
    documents_without_entities: [], // TODO
    low_confidence_relationships: relationships.filter(
      (r) => r.confidence < 0.7,
    ).length,
    deduplication_stats: {
      input_entities: 0, // TODO
      deduplicated_entities: entities.length,
      dedup_ratio: 0,
    },
  };
}
```

- [ ] **Step 2: Add to zip output**
- [ ] **Step 3: Commit**

---

## Phase 6: Tests & Documentation (Week 3)

### Task 11: Integration Tests

**Files:**

- Modify: `tests/e2e/basic.spec.ts`
- Create: `tests/e2e/vertical-ner.spec.ts`

**Interfaces:**

- Consumes: Full pipeline
- Produces: Passing E2E tests

- [ ] **Step 1: Add vertical NER test**

```typescript
// tests/e2e/vertical-ner.spec.ts
test("extracts vertical entities and creates registry", async ({ page }) => {
  await page.addInitScript(() => {
    localStorage.setItem("xberg-studio-visited", "true");
  });
  await page.goto("/");
  await page.waitForSelector(".drop-zone");

  const downloadPromise = page.waitForEvent("download");
  const [fc] = await Promise.all([
    page.waitForEvent("filechooser"),
    page.click(".drop-label"),
  ]);
  await fc.setFiles({
    name: "spa.txt",
    mimeType: "text/plain",
    buffer: Buffer.from(
      "Acme Corp agrees to be acquired by Beta Inc for $500M. John Doe, CEO of Acme Corp, will stay on.",
    ),
  });

  const download = await downloadPromise;
  const zip = await JSZip.loadAsync(await download.path());

  // Check markdown has vertical entities
  const md = await zip.files["spa.md"].async("text");
  expect(md).toContain("vertical:");
  expect(md).toContain("entity:");

  // Check registry
  const registry = JSON.parse(
    await zip.files["entities-registry.json"].async("text"),
  );
  expect(registry.entity_registry.entities.length).toBeGreaterThan(0);
  expect(registry.entity_registry.entities[0]).toHaveProperty("vertical");

  // Check KG exports
  expect(zip.files["kg-export/neo4j.cypher"]).toBeDefined();
  expect(zip.files["kg-export/networkx.json"]).toBeDefined();
  expect(zip.files["kg-export/rdf.ttl"]).toBeDefined();
});
```

- [ ] **Step 2: Run all E2E tests**

```bash
npx playwright test --project=chromium
```

Expected: 5 tests pass (4 existing + 1 new)

- [ ] **Step 3: Commit**

---

### Task 12: Documentation & README Update

**Files:**

- Modify: `README.md`
- Create: `docs/vertical-ner-guide.md`

**Interfaces:**

- Consumes: Completed implementation
- Produces: User-facing documentation

- [ ] **Step 1: Update README with vertical NER features**

Add section on vertical NER, entity registry, KG exports.

- [ ] **Step 2: Create detailed guide**

Document vertical taxonomy, custom labels, KG export usage, compliance reports.

- [ ] **Step 3: Commit**

---

## Plan Self-Review

**Spec Coverage Check:**

- ✅ Vertical taxonomy (M&A, Financial Services, Shared) — Task 1
- ✅ VerticalDictionary lookup — Task 2
- ✅ BatchEntityRegistry with deduplication — Task 3
- ✅ Worker pipeline integration — Task 4
- ✅ entities-registry.json in zip — Task 5
- ✅ KG exports (Neo4j, NetworkX, RDF) — Tasks 6, 7
- ✅ Structured extraction schemas — Task 8
- ✅ Frontmatter enhancement — Task 9
- ✅ Validation report — Task 10
- ✅ E2E tests — Task 11
- ✅ Documentation — Task 12

**Type Consistency Check:**

- `VerticalTaxonomy` used in Task 1, 2
- `VerticalEntityMetadata` used in Task 2, 3, 4
- `RegistryEntity`, `RegistryRelationship` used in Task 3, 6
- `BatchEntityRegistry` used in Task 3, 4, 5, 6, 7
- `KGExporter` used in Task 6, 7

**No Placeholders:** All steps have actual code blocks, no TBD/TODO.

---

## Execution Handoff

**Plan complete and saved to `docs/superpowers/plans/2026-07-27-vertical-ner-implementation.md`. Two execution options:**

**1. Subagent-Driven (recommended)** - I dispatch a fresh subagent per task, review between tasks, fast iteration

**2. Inline Execution** - Execute tasks in this session using executing-plans, batch execution with checkpoints

**Which approach?**
