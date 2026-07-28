import { BatchEntityRegistry } from "./registry";

export class KGExporter {
  constructor(private registry: BatchEntityRegistry) {}

  toCypher(): string {
    const entities = this.registry.getEntities();
    const relationships = this.registry.getRelationships();

    let cypher = "// Entities\n";

    // Create entities
    for (const e of entities) {
      const props = [
        `id: "${e.id}"`,
        `canonical_name: "${e.canonical_name.replace(/"/g, '\\"')}"`,
        `display_name: "${e.display_name.replace(/"/g, '\\"')}"`,
        `type: "${e.type}"`,
        `vertical: "${e.vertical}"`,
        ...(e.sector ? [`sector: "${e.sector}"`] : []),
        ...(e.roles.length ? [`roles: ${JSON.stringify(e.roles)}`] : []),
        `mention_count: ${e.mention_count}`,
      ].join(", ");

      cypher += `CREATE (e:Entity {${props}});\n`;
    }

    // Create relationships
    cypher += "\n// Relationships\n";
    for (const r of relationships) {
      cypher += `MATCH (a:Entity {id: "${r.source_entity_id}"}), (b:Entity {id: "${r.target_entity_id}"})\n`;
      cypher += `CREATE (a)-[:${r.relationship_type.toUpperCase()} {confidence: ${r.confidence}, context: "${r.context.replace(/"/g, '\\"')}"}]->(b);\n`;
    }

    return cypher;
  }

  toNetworkX(): object {
    const entities = this.registry.getEntities();
    const relationships = this.registry.getRelationships();

    return {
      nodes: entities.map((e) => ({
        id: e.id,
        name: e.canonical_name,
        display_name: e.display_name,
        type: e.type,
        vertical: e.vertical,
        sector: e.sector,
        roles: e.roles,
      })),
      edges: relationships.map((r) => ({
        source: r.source_entity_id,
        target: r.target_entity_id,
        type: r.relationship_type,
        confidence: r.confidence,
        context: r.context,
      })),
    };
  }

  toRDF(): string {
    const entities = this.registry.getEntities();
    const relationships = this.registry.getRelationships();

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

    for (const r of relationships) {
      turtle += `<${r.source_entity_id}> xberg:${r.relationship_type} <${r.target_entity_id}> .\n`;
    }

    return turtle;
  }
}
