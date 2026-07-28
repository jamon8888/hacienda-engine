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

  const response = await fetch(`/src/lib/verticals/${vertical}.yaml`);
  if (!response.ok) {
    throw new Error(`Failed to load vertical taxonomy: ${vertical}`);
  }
  const yamlText = await response.text();
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
