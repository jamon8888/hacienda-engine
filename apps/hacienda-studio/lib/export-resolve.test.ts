/**
 * Task 5.3's explicit check (plan §5.3, last bullet): `reExportMarkdown` reconstructs
 * exported markdown by slicing `result.markdown`'s frontmatter with a regex and its
 * trailing "static" content (everything from `lastIndexOf("\n## Entities\n\n")` onward)
 * as one carried-over-unchanged block. Task 5.1 changed frontmatter to multi-line YAML;
 * Task 5.2 appends a "## Related documents" section *after* "## Entities". Both changes
 * were made deliberately compatible with this slicing (see `related-documents.ts`'s
 * `buildRelatedDocumentsSection` doc comment) — these tests prove it, rather than assert
 * it from reasoning alone.
 */
import { describe, it, expect } from "vitest";
import { reExportMarkdown, resolveExportContent } from "./export-resolve";
import type { ProcessedFile } from "./types";
import type { PiiEntity } from "./pii-engine";

function piiEntity(overrides: Partial<PiiEntity> = {}): PiiEntity {
  return {
    category: "email",
    text: "",
    start: 0,
    end: 0,
    confidence: 1,
    source: "regex",
    format_preserving: true,
    redact_template: "[EMAIL]",
    ...overrides,
  };
}

describe("reExportMarkdown: frontmatter slicing survives the YAML format (Task 5.1)", () => {
  it("still isolates a multi-line YAML frontmatter block correctly", () => {
    const rawMarkdown = "Contact jean@acme.example for details.";
    const result: ProcessedFile = {
      name: "doc.md",
      rawMarkdown,
      markdown: `---
source: doc.pdf
type: pdf
processed: 2026-08-25T00:00:00.000Z
pii_entities_found: 1
doc_type: shared
entity_ids: []
---
Contact [jean@acme.example](../entities/email-abc123.md) for details.
## Entities

- [jean@acme.example](../entities/email-abc123.md) \`Email\` — mentioned 1 time
`,
      entities: [],
      piiFindings: [],
      frontmatter: { source: "doc.pdf", type: "pdf", processed: "", piiEntitiesFound: 1, entities: [] },
    };

    const reExported = reExportMarkdown(result, []);

    expect(reExported.startsWith("---\nsource: doc.pdf")).toBe(true);
    expect(reExported).toContain("pii_entities_found: 1");
    expect(reExported).toContain("doc_type: shared");
  });
});

describe("reExportMarkdown: the \"## Related documents\" section survives a redaction-edit re-export (Task 5.2)", () => {
  const rawMarkdown = "Contact jean@acme.example for details.";
  const originalMarkdown = `---
source: doc.pdf
type: pdf
processed: 2026-08-25T00:00:00.000Z
pii_entities_found: 1
---
Contact [jean@acme.example](../entities/email-abc123.md) for details.
## Entities

- [jean@acme.example](../entities/email-abc123.md) \`Email\` — mentioned 1 time

## Related documents

- [2024/amendment.md](../2024/amendment.md) — shares Acme SAS
`;

  it("carries the Related documents section through unchanged when findings are re-spliced", () => {
    const result: ProcessedFile = {
      name: "doc.md",
      rawMarkdown,
      markdown: originalMarkdown,
      entities: [],
      piiFindings: [],
      frontmatter: { source: "doc.pdf", type: "pdf", processed: "", piiEntitiesFound: 1, entities: [] },
    };
    const editedFindings = [
      piiEntity({ start: rawMarkdown.indexOf("jean@acme.example"), end: rawMarkdown.indexOf("jean@acme.example") + "jean@acme.example".length }),
    ];

    const reExported = reExportMarkdown(result, editedFindings);
    const bodyOnly = reExported.slice(0, reExported.indexOf("## Entities"));

    // The body was genuinely re-spliced against the edited findings (redacted, not
    // linked, since renderAnnotatedMarkdown has no entities to link here).
    expect(bodyOnly).toContain("[EMAIL]");
    expect(bodyOnly).not.toContain("jean@acme.example");
    // The trailing "## Entities" + "## Related documents" block is carried over
    // byte-for-byte from the ORIGINAL export, per this function's own documented scope
    // cut — including the stale entity glossary link (still naming the real address),
    // which is *expected*, pre-existing staleness, not something this task changes.
    expect(reExported).toContain("## Related documents");
    expect(reExported).toContain("[2024/amendment.md](../2024/amendment.md) — shares Acme SAS");
  });

  it("resolveExportContent's plain (no override) path returns markdown with the section intact", () => {
    const result: ProcessedFile = {
      name: "doc.md",
      rawMarkdown,
      markdown: originalMarkdown,
      entities: [],
      piiFindings: [],
      frontmatter: { source: "doc.pdf", type: "pdf", processed: "", piiEntitiesFound: 1, entities: [] },
    };

    const resolved = resolveExportContent(result, new Map(), new Map());

    expect(resolved).toBe(originalMarkdown);
    expect(resolved).toContain("## Related documents");
  });

  it("resolveExportContent's editedFindings path (Track I4) preserves the section via reExportMarkdown", () => {
    const result: ProcessedFile = {
      name: "doc.md",
      rawMarkdown,
      markdown: originalMarkdown,
      entities: [],
      piiFindings: [],
      frontmatter: { source: "doc.pdf", type: "pdf", processed: "", piiEntitiesFound: 1, entities: [] },
    };
    const editedFindings = new Map([["doc.md", [piiEntity({ start: 8, end: 25 })]]]);

    const resolved = resolveExportContent(result, new Map(), editedFindings);

    expect(resolved).toContain("## Related documents");
  });

  it("resolveExportContent's redactedDrafts path (Track K2) is a free-text override that legitimately does NOT carry the section", () => {
    // K2's whole point is a free-text buffer independent of the original export's
    // structure — this is documented pre-existing behavior, not something Task 5.2
    // changes or needs to preserve.
    const result: ProcessedFile = {
      name: "doc.md",
      rawMarkdown,
      markdown: originalMarkdown,
      entities: [],
      piiFindings: [],
      frontmatter: { source: "doc.pdf", type: "pdf", processed: "", piiEntitiesFound: 1, entities: [] },
    };
    const drafts = new Map([["doc.md", "manually edited content, no trailing sections"]]);

    const resolved = resolveExportContent(result, drafts, new Map());

    expect(resolved).toBe("manually edited content, no trailing sections");
  });
});
