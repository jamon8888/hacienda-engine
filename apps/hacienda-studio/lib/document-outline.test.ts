import { describe, it, expect } from "vitest";
import { extractHeadings } from "./document-outline";
import type { PiiEntity } from "./pii-engine";

function finding(start: number, end: number): PiiEntity {
  return { category: "email", text: "x".repeat(end - start), start, end };
}

describe("extractHeadings", () => {
  it("returns nothing for markdown with no ATX headings", () => {
    expect(extractHeadings("just some text\nmore text", [])).toEqual([]);
  });

  it("extracts headings with their level and text", () => {
    const markdown = "# Title\n\nsome intro\n\n## Section One\n\nbody\n\n### Sub\n\nmore body";
    const headings = extractHeadings(markdown, []);
    expect(headings.map((h) => [h.level, h.text])).toEqual([
      [1, "Title"],
      [2, "Section One"],
      [3, "Sub"],
    ]);
  });

  it("ignores non-ATX lines that merely contain a #", () => {
    const markdown = "# Real Heading\n\nText with a # in the middle, not a heading.";
    expect(extractHeadings(markdown, [])).toHaveLength(1);
  });

  it("attributes each finding's count to the section it falls inside", () => {
    const markdown = "# A\n\nemail1\n\n## B\n\nemail2\nemail3";
    const emailAStart = markdown.indexOf("email1");
    const emailBStart = markdown.indexOf("email2");
    const emailB2Start = markdown.indexOf("email3");
    const findings = [
      finding(emailAStart, emailAStart + 6),
      finding(emailBStart, emailBStart + 6),
      finding(emailB2Start, emailB2Start + 6),
    ];
    const headings = extractHeadings(markdown, findings);
    expect(headings.find((h) => h.text === "A")?.piiCount).toBe(1);
    expect(headings.find((h) => h.text === "B")?.piiCount).toBe(2);
  });

  it("attributes the last section's findings through end of document", () => {
    const markdown = "# Only\n\ntrailing content with a finding here";
    const start = markdown.indexOf("finding");
    const headings = extractHeadings(markdown, [finding(start, start + 7)]);
    expect(headings[0].piiCount).toBe(1);
  });

  it("does not count a finding that falls before the first heading", () => {
    const markdown = "preamble text\n\n# Heading\n\nbody";
    const headings = extractHeadings(markdown, [finding(0, 5)]);
    expect(headings[0].piiCount).toBe(0);
  });
});
