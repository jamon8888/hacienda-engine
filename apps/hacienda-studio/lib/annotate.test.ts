import { describe, it, expect } from "vitest";
import { spliceRedaction } from "./annotate";

describe("spliceRedaction", () => {
  it("replaces the selected range with an uppercased category token", () => {
    const text = "Contact Jean Dupont for details.";
    const from = text.indexOf("Jean Dupont");
    const to = from + "Jean Dupont".length;

    expect(spliceRedaction(text, from, to, "person")).toBe(
      "Contact [PERSON] for details.",
    );
  });

  it("leaves text outside the selection untouched", () => {
    const text = "abcdef";
    expect(spliceRedaction(text, 2, 4, "other")).toBe("ab[OTHER]ef");
  });

  it("inserts at a collapsed selection without deleting anything", () => {
    const text = "abcdef";
    expect(spliceRedaction(text, 3, 3, "email")).toBe("abc[EMAIL]def");
  });
});
