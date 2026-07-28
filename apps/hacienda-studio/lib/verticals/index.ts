import maYaml from "./m&a.yaml?raw";
import financialServicesYaml from "./financial_services.yaml?raw";
import sharedYaml from "./shared.yaml?raw";

/**
 * Taxonomies are bundled at build time rather than fetched at runtime. The
 * previous fetch("/src/lib/verticals/…") hit the SPA fallback, which returns
 * index.html with HTTP 200 — so response.ok was true, HTML reached parseYAML,
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

let taxonomyCache: Map<string, VerticalTaxonomy> = new Map();

export async function loadVerticalTaxonomy(
  vertical: string,
): Promise<VerticalTaxonomy> {
  if (taxonomyCache.has(vertical)) {
    return taxonomyCache.get(vertical)!;
  }

  const yamlText = RAW_TAXONOMIES[vertical];
  if (yamlText === undefined) {
    throw new Error(`Unknown vertical taxonomy: ${vertical}`);
  }
  const taxonomy = parseYAML(yamlText) as VerticalTaxonomy;
  taxonomy.vertical = vertical;
  taxonomyCache.set(vertical, taxonomy);
  return taxonomy;
}

function parseYAML(yamlText: string): Record<string, any> {
  const lines = yamlText.split("\n");
  const result: Record<string, any> = {};
  let currentKey: string | null = null;
  let currentArray: string[] | null = null;

  for (const line of lines) {
    const trimmed = line.trim();
    if (!trimmed || trimmed.startsWith("#")) continue;

    const indent = line.length - line.trimStart().length;
    const isArrayItem = trimmed.startsWith("- ");

    if (isArrayItem) {
      const value = trimmed.substring(2).trim();
      if (currentArray !== null) {
        currentArray.push(value);
      }
      continue;
    }

    if (trimmed.includes(":")) {
      const [key, ...rest] = trimmed.split(":");
      const value = rest.join(":").trim();
      currentKey = key.trim();

      if (value === "") {
        currentArray = [];
        result[currentKey] = currentArray;
      } else {
        result[currentKey] = value;
        currentArray = null;
      }
    }
  }

  return result;
}

export function loadVerticalTaxonomySync(vertical: string): VerticalTaxonomy {
  if (taxonomyCache.has(vertical)) {
    return taxonomyCache.get(vertical)!;
  }
  // For synchronous loading in worker, we'll need to use dynamic import
  throw new Error("Use loadVerticalTaxonomy async version");
}
