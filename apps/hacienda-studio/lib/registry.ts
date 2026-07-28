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
  private entityKeyMap: Map<string, string> = new Map();
  private docEntityMap: Map<string, string[]> = new Map();

  constructor() {
    this.batchId = `batch-${Date.now()}-${Math.random().toString(36).slice(2, 7)}`;
  }

  addEntity(
    entity: {
      name: string;
      type: string;
      slug: string;
      count: number;
      spans: Array<{ start: number; end: number }>;
    },
    metadata: { vertical: string; sector?: string; roles?: string[] },
    docId: string,
  ): {
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
  } {
    // Normalize entity name for deduplication (remove trailing punctuation)
    const normalizedName = entity.name.replace(/[.,;:]+$/, "").toLowerCase();
    const key = `${normalizedName}|${entity.type}|${metadata.vertical}`;
    let existingId = this.entityKeyMap.get(key);

    if (existingId) {
      const existing = this.entities.get(existingId)!;
      if (!existing.source_documents.includes(docId)) {
        existing.source_documents.push(docId);
      }
      existing.mention_count++;
      existing.last_seen = new Date().toISOString();
      return existing;
    }

    const id = `ent-${String(this.entities.size + 1).padStart(3, "0")}`;
    const registryEntity = {
      id,
      canonical_name: entity.name,
      display_name: entity.name,
      type: entity.type,
      vertical: metadata.vertical,
      sector: metadata.sector,
      roles: metadata.roles || [],
      aliases: [],
      source_documents: [docId],
      mention_count: 1,
      vertical_metadata: metadata,
      first_seen: new Date().toISOString(),
      last_seen: new Date().toISOString(),
    };

    this.entities.set(id, registryEntity);
    this.entityKeyMap.set(key, id);

    // Track document -> entity mapping
    if (!this.docEntityMap.has(docId)) {
      this.docEntityMap.set(docId, []);
    }
    this.docEntityMap.get(docId)!.push(id);

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

  // Infer relationships based on co-occurrence and entity types
  inferRelationships(docId: string) {
    const entityIds = this.docEntityMap.get(docId) || [];
    const entities = entityIds
      .map((id) => this.entities.get(id))
      .filter(Boolean) as RegistryEntity[];

    // Infer relationships based on co-occurrence and entity types
    for (let i = 0; i < entities.length; i++) {
      for (let j = i + 1; j < entities.length; j++) {
        const e1 = entities[i];
        const e2 = entities[j];

        // Person -> Organization: works_for, officer_of
        if (e1.type === "person" && e2.type === "organization") {
          this.addRelationship(
            e1.id,
            e2.id,
            "works_for",
            `Co-occurs in document`,
            0.7,
            "inferred",
          );
        } else if (e1.type === "organization" && e2.type === "person") {
          this.addRelationship(
            e2.id,
            e1.id,
            "works_for",
            `Co-occurs in document`,
            0.7,
            "inferred",
          );
        }

        // Organization -> Organization: subsidiary_of, parent_of, partner_of
        if (e1.type === "organization" && e2.type === "organization") {
          this.addRelationship(
            e1.id,
            e2.id,
            "partner_of",
            `Co-occurs in document`,
            0.5,
            "inferred",
          );
          this.addRelationship(
            e2.id,
            e1.id,
            "partner_of",
            `Co-occurs in document`,
            0.5,
            "inferred",
          );
        }

        // Person -> Email: contact_email
        if (e1.type === "person" && e2.type === "email") {
          this.addRelationship(
            e1.id,
            e2.id,
            "contact_email",
            `Email associated with person`,
            0.8,
            "inferred",
          );
        } else if (e1.type === "email" && e2.type === "person") {
          this.addRelationship(
            e2.id,
            e1.id,
            "contact_email",
            `Email associated with person`,
            0.8,
            "inferred",
          );
        }

        // Organization -> Email: contact_email
        if (e1.type === "organization" && e2.type === "email") {
          this.addRelationship(
            e1.id,
            e2.id,
            "contact_email",
            `Contact email for organization`,
            0.7,
            "inferred",
          );
        } else if (e1.type === "email" && e2.type === "organization") {
          this.addRelationship(
            e2.id,
            e1.id,
            "contact_email",
            `Contact email for organization`,
            0.7,
            "inferred",
          );
        }
      }
    }
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
      document_entities: Object.fromEntries(this.docEntityMap),
      vertical_summary: this.getVerticalSummary(),
    };
  }

  getVerticalSummary() {
    const summary: Record<
      string,
      { entity_count: number; relationship_count: number }
    > = {};
    for (const e of this.entities.values()) {
      if (!summary[e.vertical]) {
        summary[e.vertical] = { entity_count: 0, relationship_count: 0 };
      }
      summary[e.vertical].entity_count++;
    }
    for (const r of this.relationships) {
      const sourceEntity = this.entities.get(r.source_entity_id);
      if (sourceEntity && summary[sourceEntity.vertical]) {
        summary[sourceEntity.vertical].relationship_count++;
      }
    }
    return summary;
  }

  getEntities() {
    return Array.from(this.entities.values());
  }
  getRelationships() {
    return this.relationships;
  }
  getBatchId() {
    return this.batchId;
  }
}
