import { parse } from "yaml";
import maYaml from "./m&a.yaml?raw";
import financialServicesYaml from "./financial_services.yaml?raw";
import sharedYaml from "./shared.yaml?raw";

/**
 * Taxonomies are bundled at build time rather than fetched at runtime. The
 * previous fetch("/src/lib/verticals/…") hit the SPA fallback, which returns
 * index.html with HTTP 200 — so response.ok was true, HTML reached the parser,
 * and the resulting taxonomy had no entityTypes, crashing the worker on the
 * first file processed.
 */
const RAW_TAXONOMIES: Record<string, string> = {
  "m&a": maYaml,
  financial_services: financialServicesYaml,
  shared: sharedYaml,
};

export interface VerticalTaxonomy {
  vertical: string;
  sectors: string[];
  entityTypes: string[];
  relationships: string[];
}

export interface VerticalEntityMetadata {
  canonical: string;
  vertical: string;
  sector?: string;
  roles?: string[];
  aliases?: string[];
}

const taxonomyCache: Map<string, VerticalTaxonomy> = new Map();

export async function loadVerticalTaxonomy(
  vertical: string,
): Promise<VerticalTaxonomy> {
  const cached = taxonomyCache.get(vertical);
  if (cached) return cached;

  const yamlText = RAW_TAXONOMIES[vertical];
  if (yamlText === undefined) {
    throw new Error(`Unknown vertical taxonomy: ${vertical}`);
  }

  const taxonomy = parseTaxonomy(vertical, yamlText);
  taxonomyCache.set(vertical, taxonomy);
  return taxonomy;
}

function parseTaxonomy(vertical: string, yamlText: string): VerticalTaxonomy {
  const parsed = parse(yamlText) as Partial<VerticalTaxonomy> | null;
  if (parsed === null || typeof parsed !== "object") {
    throw new Error(`Taxonomy ${vertical} is not a YAML mapping`);
  }

  return {
    vertical,
    sectors: requireStringList(vertical, "sectors", parsed.sectors),
    entityTypes: requireStringList(vertical, "entityTypes", parsed.entityTypes),
    relationships: requireStringList(
      vertical,
      "relationships",
      parsed.relationships,
    ),
  };
}

/**
 * A taxonomy whose lists silently come back as something other than a list of
 * strings breaks the consumers far from here — the previous hand-rolled parser
 * read `sectors: []` as the string "[]", so `sectors[0]` was "[".
 */
function requireStringList(
  vertical: string,
  field: string,
  value: unknown,
): string[] {
  if (!Array.isArray(value) || value.some((item) => typeof item !== "string")) {
    throw new Error(
      `Taxonomy ${vertical}: "${field}" must be a list of strings, got ${JSON.stringify(value)}`,
    );
  }
  return value;
}
