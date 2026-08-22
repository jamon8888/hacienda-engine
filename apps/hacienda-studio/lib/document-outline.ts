import type { PiiEntity } from "./pii-engine";

export interface OutlineHeading {
  level: number;
  text: string;
  /** Byte offset of the heading line's first character in `rawMarkdown`. */
  start: number;
  /** Byte offset one past the heading line's last character. */
  end: number;
  /** PII findings whose `start` falls within this heading's section (up to the next
   * heading of any level, or end of document). */
  piiCount: number;
}

const HEADING_LINE = /^(#{1,6})\s+(.+?)\s*$/;

/**
 * Extract an ATX-heading table of contents from `markdown`, each entry annotated with
 * how many `findings` fall inside that heading's section. `findings[].start` is a byte
 * offset into the same string as `markdown` (both trace back to `rawMarkdown` — see
 * `ProcessedFile`'s doc comment in `lib/types.ts`), so no offset translation is needed.
 *
 * A plain per-line regex rather than a full markdown AST parse: this only needs ATX
 * heading lines, not general document structure, and the rest of this codebase's
 * document views (`CodeLines`, `PiiAnnotatedView`) already treat `rawMarkdown` as plain
 * text rather than parsing it.
 */
export function extractHeadings(markdown: string, findings: PiiEntity[]): OutlineHeading[] {
  const lines = markdown.split("\n");
  const positions: Array<{ level: number; text: string; lineStart: number; lineEnd: number }> = [];

  let offset = 0;
  for (const line of lines) {
    const lineEnd = offset + line.length;
    const match = HEADING_LINE.exec(line);
    if (match) {
      positions.push({ level: match[1].length, text: match[2], lineStart: offset, lineEnd });
    }
    // `split("\n")` consumes the newline itself; put it back for the next line's offset.
    offset = lineEnd + 1;
  }

  return positions.map((heading, i) => {
    const sectionEnd = i + 1 < positions.length ? positions[i + 1].lineStart : markdown.length;
    const piiCount = findings.filter((f) => f.start >= heading.lineEnd && f.start < sectionEnd).length;
    return {
      level: heading.level,
      text: heading.text,
      start: heading.lineStart,
      end: heading.lineEnd,
      piiCount,
    };
  });
}
