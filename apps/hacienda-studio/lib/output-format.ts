import type { AppConfig, ProcessedFile } from "./types";

export type OutputFormat = AppConfig["outputFormat"];

function bodyWithoutMetadata(markdown: string): string {
  const withoutFrontmatter = markdown.replace(/^---\n[\s\S]*?\n---\n?/, "");
  const glossaryStart = withoutFrontmatter.lastIndexOf("\n## Entities\n\n");
  return glossaryStart === -1
    ? withoutFrontmatter
    : withoutFrontmatter.slice(0, glossaryStart);
}

/** Convert the redacted Markdown body to readable text without exposing source bytes. */
export function markdownToPlainText(markdown: string): string {
  return bodyWithoutMetadata(markdown)
    .replace(/!\[([^\]]*)\]\([^)]*\)/g, "$1")
    .replace(/\[([^\]]+)\]\([^)]*\)/g, "$1")
    .replace(/^#{1,6}\s*/gm, "")
    .replace(/^\s*[-*+]\s+/gm, "")
    .replace(/^\s*>\s?/gm, "")
    .replace(/`([^`]+)`/g, "$1")
    .replace(/\*\*([^*]+)\*\*/g, "$1")
    .replace(/__([^_]+)__/g, "$1")
    .replace(/\*([^*]+)\*/g, "$1")
    .replace(/_([^_]+)_/g, "$1")
    .replace(/\n{3,}/g, "\n\n")
    .trim() + "\n";
}

/**
 * Format an already-redacted document for download. `content` may be a user's edited
 * redacted draft, so formatting is intentionally applied after all redaction edits.
 * The JSON representation contains no `rawMarkdown` field by design.
 */
export function formatExportContent(
  result: ProcessedFile,
  content: string,
  format: OutputFormat,
): string {
  switch (format) {
    case "plain":
      return markdownToPlainText(content);
    case "json":
      return JSON.stringify(
        {
          name: result.name,
          source: result.frontmatter.source,
          type: result.frontmatter.type,
          processed: result.frontmatter.processed,
          piiEntitiesFound: result.frontmatter.piiEntitiesFound,
          entities: result.entities,
          content,
        },
        null,
        2,
      ) + "\n";
    case "markdown":
    default:
      return content;
  }
}

export function outputFileName(name: string, format: OutputFormat): string {
  const stem = name.replace(/\.[^.]+$/, "");
  return format === "markdown" ? name : `${stem}.${format === "plain" ? "txt" : "json"}`;
}
