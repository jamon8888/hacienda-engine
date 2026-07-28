# Vertical NER Architecture for M&A / Corporate Law & Financial Services

**Date:** 2026-07-27  
**Status:** Approved  
**Author:** xberg-studio team

---

## 1. Problem Statement

xberg-studio currently extracts base entities (Person, Organization, Location, Email, Date, Money) via GLiNER2 + Compromise bridge. For M&A/Corporate Law and Financial Services verticals, we need:

- **Vertical-specific entities** (deal_type, earnout, representation_and_warranty, fund, portfolio_company, etc.)
- **Entity enrichment** with vertical metadata (sector, role, vertical taxonomy)
- **Batch-level entity registry** for deduplication across documents
- **Knowledge graph export** for RAG traversal, compliance, deal analytics

---

## 2. Architecture Overview

```text
┌─────────────────────────────────────────────────────────────────────────┐
│                        VERTICAL NER PIPELINE                            │
├─────────────────────────────────────────────────────────────────────────┤
│                                                                         │
│  Document ──▶ XbergEngine.extract() ──▶ Base NER (GLiNER2 + Compromise)│
│       │                                          │                     │
│       ▼                                          ▼                     │
│  Structured Extraction ◀── Vertical Dictionary ◀── Taxonomy Mapper    │
│       │                                          │                     │
│       ▼                                          ▼                     │
│  Entity Graph Builder ──▶ Batch Registry ◀── Vertical Taxonomy       │
│       │                                          │                     │
│       ▼                                          ▼                     │
│  Outputs: Markdown + Registry + KG Export (Neo4j/NetworkX/RDF)       │
│                                                                         │
└─────────────────────────────────────────────────────────────────────────┘
```

### Data Flow

1. **Extraction** — `XbergEngine.extract()` with `WasmStructuredExtractionConfig`
2. **Base NER** — GLiNER2 (Onnx) + Compromise bridge for base entities
3. **Vertical Dictionary Lookup** — Match extracted terms against vertical taxonomy
4. **Structured Extraction** — `WasmStructuredExtractionConfig` for vertical fields
5. **Taxonomy Mapping** — Enrich entities with vertical metadata (sector, role, vertical)
6. **Entity Graph** — Build nodes + edges for knowledge graph
7. **Batch Registry** — Deduplicated entity store per batch
8. **Exports** — Markdown + Registry JSON + KG exports (Neo4j/NetworkX/RDF)

---

## 3. Vertical Taxonomy

### M&A / Corporate Law

```yaml
vertical: m&a
sectors:
  - technology
  - healthcare
  - industrials
  - consumer
  - financial_services
  - energy
  - real_estate

entity_types:
  # Deal parties
  - target_company
  - acquirer
  - seller
  - financial_advisor
  - legal_advisor
  - auditor
  # Deal structure
  - deal_type:
      [
        merger,
        acquisition,
        divestiture,
        carve_out,
        joint_venture,
        spinoff,
        management_buyout,
      ]
  - deal_value
  - earnout
  - escrow
  - purchase_price_adjustment
  # Legal terms
  - representation_and_warranty
  - indemnification
  - material_adverse_change
  - closing_condition
  - break_fee
  - reverse_break_fee
  - governing_law
  - jurisdiction
  # Financial
  - debt_financing
  - equity_financing
  - rollover_equity
  - seller_note
  - earnout_milestone
  # Entities
  - target_company
  - acquirer
  - seller
  - financial_advisor
  - legal_advisor
  - regulator
  - portfolio_company

relationships:
  - acquirer_of
  - target_of
  - advised_by (financial/legal)
  - party_to (agreement)
  - subsidiary_of
  - parent_of
  - guarantor_of
```

### Financial Services (Banking, PE/VC, Capital Markets)

```yaml
vertical: financial_services
sectors:
  - banking
  - pe_vc
  - capital_markets
  - asset_management
  - insurance
  - fintech

entity_types:
  # PE/VC
  - fund
  - portfolio_company
  - limited_partner (LP)
  - general_partner (GP)
  - investment_commitment
  - management_fee
  - carried_interest
  - nav
  - irr
  - dpi
  - tvpi
  # Banking
  - borrower
  - lender
  - agent_bank
  - syndicate_member
  - facility_amount
  - interest_rate
  - maturity_date
  - covenant
  # Capital Markets
  - issuer
  - underwriter
  - bookrunner
  - offering_size
  - coupon
  - maturity
  # Entities
  - fund
  - portfolio_company
  - limited_partner
  - general_partner
  - borrower
  - lender
  - issuer
  - underwriter
  - trustee

relationships:
  - invests_in
  - manages
  - limited_partner_of
  - general_partner_of
  - lender_to
  - borrower_from
  - underwrites
  - guarantees
  - securitizes
```

### Cross-Vertical (Shared)

```yaml
shared:
  entity_types:
    - person
    - organization
    - location
    - email
    - phone_number
    - date
    - money
    - percentage
    - url
    - legal_entity
    - contract
    - agreement
    - regulation
    - regulator
  relationships:
    - works_for
    - located_in
    - party_to
    - subsidiary_of
    - parent_of
    - officer_of
    - director_of
    - shareholder_of
```

---

## 4. Batch Entity Registry

### Output: `entities-registry.json`

```json
{
  "batch_id": "batch-2026-07-27-001",
  "processed_at": "2026-07-27T15:00:00Z",
  "total_documents": 42,
  "vertical_taxonomy_version": "1.0.0",
  "entity_registry": {
    "entities": [
      {
        "id": "ent-001",
        "canonical_name": "Acme Corporation",
        "display_name": "Acme Corp",
        "type": "Organization",
        "vertical": "m&a",
        "sector": "technology",
        "roles": ["target_company"],
        "aliases": ["Acme", "Acme Inc", "Acme Corp"],
        "source_documents": ["doc-001", "doc-005"],
        "mention_count": 23,
        "vertical_metadata": {
          "sector": "technology",
          "roles": ["target_company"],
          "jurisdiction": "Delaware"
        },
        "first_seen": "2026-07-27T14:30:00Z",
        "last_seen": "2026-07-27T15:45:00Z"
      },
      {
        "id": "ent-002",
        "canonical_name": "John A. Doe",
        "display_name": "John Doe",
        "type": "Person",
        "vertical": "m&a",
        "sector": "technology",
        "roles": ["ceo", "key_executive", "key_person"],
        "aliases": ["John Doe", "J. Doe"],
        "source_documents": ["doc-001", "doc-003"],
        "mention_count": 12,
        "vertical_metadata": {
          "title": "CEO",
          "roles": ["ceo", "key_executive"]
        }
      }
    ],
    "relationships": [
      {
        "id": "rel-001",
        "source_entity_id": "ent-002",
        "target_entity_id": "ent-001",
        "relationship_type": "officer_of",
        "context": "CEO of Acme Corp per SPA dated 2026-01-15",
        "confidence": 0.95,
        "source_document": "doc-001"
      },
      {
        "id": "rel-002",
        "source_entity_id": "ent-003",
        "target_entity_id": "ent-001",
        "relationship_type": "advised_by",
        "sub_type": "financial_advisor",
        "context": "Goldman Sachs advised Acme Corp on sale to Beta Inc",
        "confidence": 0.92,
        "source_document": "doc-005"
      }
    ]
  },
  "document_entities": {
    "doc-001": ["ent-001", "ent-002", "ent-003"],
    "doc-002": ["ent-001", "ent-004"]
  },
  "vertical_summary": {
    "m&a": { "entity_count": 142, "relationship_count": 87 },
    "financial_services": { "entity_count": 56, "relationship_count": 34 }
  }
}
```

---

## 5. Knowledge Graph Exports

### Neo4j Cypher

```cypher
// Entities
CREATE (e:Entity {
  id: "ent-001",
  canonical_name: "Acme Corporation",
  display_name: "Acme Corp",
  type: "Organization",
  vertical: "m&a",
  sector: "technology",
  roles: ["target_company"],
  aliases: ["Acme", "Acme Inc"],
  mention_count: 23
});

// Relationships
MATCH (a:Entity {id: "ent-002"}), (b:Entity {id: "ent-001"})
CREATE (a)-[:OFFICER_OF {confidence: 0.95, context: "CEO of Acme Corp per SPA dated 2026-01-15"}]->(b);
```

### NetworkX JSON

```json
{
  "nodes": [
    {
      "id": "ent-001",
      "name": "Acme Corporation",
      "type": "Organization",
      "vertical": "m&a",
      "sector": "technology",
      "roles": ["target_company"]
    },
    {
      "id": "ent-002",
      "name": "John Doe",
      "type": "Person",
      "vertical": "m&a",
      "roles": ["ceo"]
    }
  ],
  "edges": [
    {
      "source": "ent-002",
      "target": "ent-001",
      "type": "officer_of",
      "confidence": 0.95
    }
  ]
}
```

### RDF/Turtle

```turtle
@prefix xberg: <http://xberg.io/ontology#> .
@prefix schema: <http://schema.org/> .

<ent-001> a xberg:Organization ;
  schema:name "Acme Corporation" ;
  xberg:displayName "Acme Corp" ;
  xberg:vertical "m&a" ;
  xberg:sector "technology" ;
  xberg:role "target_company" ;
  xberg:aliases ("Acme" "Acme Inc") ;
  xberg:mentionCount 23 .

<ent-002> a xberg:Person ;
  schema:name "John A. Doe" ;
  xberg:displayName "John Doe" ;
  xberg:officerOf <ent-001> .
```

---

## 5. NER Pipeline Configuration

### Hybrid NER Configuration

```typescript
// Worker pipeline config
const extractConfig = WasmExtractionConfig.default();
extractConfig.outputFormat = WasmOutputFormat.Markdown;

// Base NER: GLiNER2 (Onnx) + Compromise bridge
const nerConfig = WasmNerConfig.default();
nerConfig.backend = WasmNerBackendKind.Onnx;
nerConfig.categories = [
  // Base
  "person", "organization", "location", "email", "phone_number", "date", "money", "percentage",
  // M&A vertical
  "legal_entity", "counterparty", "jurisdiction", "regulator",
  "contract_type", "governing_law",
  // Financial
  "financial_instrument", "fund", "portfolio_company"
];
nerConfig.customLabels = [
  // M&A
  "m&a", "private equity", "venture capital", "due diligence",
  "share purchase agreement", "asset purchase", "merger",
  "earnout", "indemnification", "representation and warranty",
  "material adverse change", "closing condition", "break fee",
  // Financial Services
  "private equity", "venture capital", "limited partner", "general partner",
  "carried interest", "management fee", "nav", "irr", "dpi", "tvpi",
  "fund", "portfolio_company", "limited_partner", "general_partner",
  "management_fee", "carried_interest", "investment_commitment"
];

// NER bridge injection for Compromise fallback
const nerBridge = async (text: string, categories: string[]) => { ... };
const engine = new XbergEngine({ bridgeTimeoutMs: 30000 }, { ner: { ner: nerBridge } });
```

---

## 6. Implementation Plan

### Phase 1: Core Vertical Dictionary & Registry (Week 1)

- [ ] Create vertical taxonomy YAML files (`m&a.yaml`, `financial_services.yaml`, `shared.yaml`)
- [ ] Build `VerticalDictionary` class for term → metadata lookup
- [ ] Implement `BatchEntityRegistry` with deduplication
- [ ] Add `entities-registry.json` output to worker pipeline

### Phase 2: Vertical Entity Enrichment (Week 1-2)

- [ ] Implement `VerticalTaxonomyMapper` — maps base entities to vertical metadata
- [ ] Add `customLabels` for M&A/Financial zero-shot terms
- [ ] Entity deduplication across batch (canonical name + type + vertical)
- [ ] Add `entity-registry.json` to zip output

### Phase 3: Knowledge Graph Export (Week 2)

- [ ] Implement `EntityGraphBuilder` — nodes + edges
- [ ] Neo4j Cypher export
- [ ] NetworkX JSON export
- [ ] RDF/Turtle export
- [ ] Add `kg-export/` folder to zip output

### Phase 4: Structured Extraction Integration (Week 2-3)

- [ ] Add `WasmStructuredExtractionConfig` for vertical field extraction
- [ ] Define extraction schemas per document type (SPA, LPA, Credit Agreement, etc.)
- [ ] Map structured fields to entity registry

### Phase 5: Compliance & RAG Features (Week 3)

- [ ] Add entity relationship extraction (source doc, context, confidence)
- [ ] Entity linking in markdown (`[Entity](entity:type/slug)`)
- [ ] Frontmatter enrichment with vertical metadata
- [ ] Batch registry validation report

---

## 6. Configuration

### Vertical Dictionary Structure

```text
src/lib/verticals/
├── m&a.yaml              # M&A taxonomy
├── financial_services.yaml
├── shared.yaml           # Cross-vertical
└── index.ts              # Loader + registry
```

### Configuration Options

```typescript
interface VerticalConfig {
  enabledVerticals: ("m&a" | "financial_services")[];
  enableKGExport: boolean;
  kgFormats: ("neo4j" | "networkx" | "rdf")[];
  enableStructuredExtraction: boolean;
  verticalDictionaryPath: string;
}
```

---

## 7. Testing Strategy

### Unit Tests

- [ ] Vertical dictionary lookup accuracy
- [ ] Entity deduplication logic
- [ ] Relationship inference rules
- [ ] KG export format validity

### Integration Tests

- [ ] Full pipeline: document → markdown + registry + KG
- [ ] Batch registry deduplication across 10+ documents
- [ ] KG export format validation (Cypher, NetworkX, RDF)

### Golden Files

- [ ] Sample M&A documents (SPA, LOI, DD reports)
- [ ] Sample Financial Services (LPA, Credit Agreement, Term Sheet)
- [ ] Expected registry/KG outputs

---

## 8. Future Extensions

| Feature                         | Timeline | Notes                                 |
| ------------------------------- | -------- | ------------------------------------- |
| Fine-tuned GLiNER2 per vertical | Q4 2026  | When training data available          |
| LLM backend for complex fields  | Q4 2026  | WasmNerBackendKind.Llm                |
| Real-time entity linking UI     | Q1 2027  | Frontend component                    |
| Multi-lingual vertical taxonomy | Q1 2027  | French/English for Droit des affaires |
| Regulatory compliance reports   | Q1 2027  | Auto-generate from registry           |

---

## 9. Acceptance Criteria

- [ ] Batch processing produces `entities-registry.json` with deduplicated entities
- [ ] Entities enriched with vertical, sector, roles, aliases
- [ ] Relationships extracted with confidence scores
- [ ] KG exports valid for Neo4j, NetworkX, RDF
- [ ] Frontmatter includes vertical metadata
- [ ] Markdown entity links work for RAG traversal
- [ ] All 4 E2E tests pass
- [ ] Build completes without errors

---

**Approved by:** xberg-studio team  
**Next step:** Invoke writing-plans skill to create implementation plan
