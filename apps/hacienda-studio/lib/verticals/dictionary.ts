import { VerticalTaxonomy, VerticalEntityMetadata } from "./index";

export class VerticalDictionary {
  private map: Map<string, VerticalEntityMetadata> = new Map();

  /**
   * `taxonomy.sectors` lists the industry sectors a vertical covers; it says
   * nothing about any individual entity type. Deriving a per-entity `sector`
   * from it stamped every M&A entity `technology`, which then propagated into
   * the exported knowledge graph as fact. Sector is left unset until a
   * document-level classifier can supply it.
   */
  constructor(taxonomies: VerticalTaxonomy[]) {
    for (const taxonomy of taxonomies) {
      for (const entityType of taxonomy.entityTypes) {
        this.map.set(entityType.toLowerCase(), {
          canonical: entityType,
          vertical: taxonomy.vertical,
        });
        // Add common aliases
        const aliases = this.getAliases(entityType);
        for (const alias of aliases) {
          this.map.set(alias.toLowerCase(), {
            canonical: entityType,
            vertical: taxonomy.vertical,
          });
        }
      }
    }
  }

  private getAliases(entityType: string): string[] {
    const aliasMap: Record<string, string[]> = {
      target_company: ["target", "target company", "target co"],
      acquirer: ["buyer", "purchaser"],
      share_purchase_agreement: [
        "spa",
        "purchase agreement",
        "purchase and sale agreement",
      ],
      asset_purchase: ["asset purchase", "asset sale"],
      earnout: ["earn out", "earn-out"],
      indemnification: ["indemnity"],
      representation_and_warranty: [
        "rep and warranty",
        "reps and warranties",
        "representations and warranties",
      ],
      material_adverse_change: ["mac", "material adverse effect"],
      closing_condition: ["condition precedent"],
      break_fee: ["termination fee", "breakup fee"],
      reverse_break_fee: ["reverse termination fee"],
      fund: [
        "pe fund",
        "vc fund",
        "private equity fund",
        "venture capital fund",
      ],
      portfolio_company: ["portco", "portfolio co"],
      limited_partner: ["lp", "limited partner"],
      general_partner: ["gp", "general partner"],
      carried_interest: ["carry"],
      management_fee: ["mgmt fee"],
      investment_commitment: ["commitment", "capital commitment"],
      nav: ["net asset value"],
      irr: ["internal rate of return"],
      dpi: ["distributions to paid-in"],
      tvpi: ["total value to paid-in"],
    };
    return aliasMap[entityType] || [];
  }

  lookup(term: string): VerticalEntityMetadata | null {
    // Clean the term: remove trailing punctuation
    const cleaned = term.toLowerCase().replace(/[.,;:]+$/, "");
    return this.map.get(cleaned) || this.map.get(term.toLowerCase()) || null;
  }

  lookupAll(terms: string[]): VerticalEntityMetadata[] {
    return terms
      .map((t) => this.lookup(t))
      .filter((v): v is VerticalEntityMetadata => v !== null);
  }
}
