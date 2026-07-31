import { describe, expect, it } from "vitest";
import { EditorState, StateField } from "@codemirror/state";
import type { DecorationSet } from "@codemirror/view";
import { buildDecorations, piiHighlightExtension } from "./pii-decorations";
import type { PiiEntity } from "./pii-engine";

function piiEntity(overrides: Partial<PiiEntity>): PiiEntity {
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

/**
 * `piiHighlightExtension` doesn't export its `StateField` directly (the module's public
 * surface is the compiled `Extension`, matching how `@uiw/react-codemirror` consumes it) —
 * reading decorations back for assertions needs the field itself, so tests build one
 * locally with the exact same `create`/`update` shape, seeded from the module's own
 * `buildDecorations` so construction isn't reimplemented a second time, just re-exposed.
 */
function makeState(
  text: string,
  findings: PiiEntity[],
): { state: EditorState; field: StateField<DecorationSet> } {
  const field = StateField.define<DecorationSet>({
    create: (s) => buildDecorations(findings, s.doc.length),
    update: (decorations, tr) => decorations.map(tr.changes),
  });
  const state = EditorState.create({ doc: text, extensions: [field] });
  return { state, field };
}

function ranges(state: EditorState, field: StateField<DecorationSet>) {
  const out: Array<{ from: number; to: number; category: string | null }> = [];
  const cursor = state.field(field).iter();
  while (cursor.value) {
    out.push({
      from: cursor.from,
      to: cursor.to,
      category: (cursor.value.spec.attributes as Record<string, string> | undefined)?.[
        "data-pii-category"
      ] ?? null,
    });
    cursor.next();
  }
  return out;
}

describe("piiHighlightExtension / buildDecorations (Track F3)", () => {
  it("compiles to a real CodeMirror extension usable by @uiw/react-codemirror", () => {
    const extension = piiHighlightExtension([
      piiEntity({ category: "email", start: 8, end: 19 }),
    ]);
    // A real EditorState must accept it without throwing — the actual integration check,
    // not just that the function returns *something*.
    expect(() => EditorState.create({ doc: "x".repeat(20), extensions: [extension] })).not.toThrow();
  });

  it("decorates the exact span each finding names", () => {
    const text = "Contact Jean Dupont at work.";
    const finding = piiEntity({ category: "person", start: 8, end: 19 }); // "Jean Dupont"
    const { state, field } = makeState(text, [finding]);
    expect(ranges(state, field)).toEqual([{ from: 8, to: 19, category: "person" }]);
    expect(text.slice(8, 19)).toBe("Jean Dupont");
  });

  it("drops out-of-bounds findings instead of throwing", () => {
    const text = "short";
    const finding = piiEntity({ start: 100, end: 110 });
    const { state, field } = makeState(text, [finding]);
    expect(ranges(state, field)).toEqual([]);
  });

  it("drops the second of two overlapping findings rather than crashing the builder", () => {
    const text = "abcdefghij";
    const a = piiEntity({ category: "email", start: 2, end: 6 });
    const b = piiEntity({ category: "phone", start: 4, end: 8 }); // overlaps a
    const { state, field } = makeState(text, [a, b]);
    expect(ranges(state, field)).toEqual([{ from: 2, to: 6, category: "email" }]);
  });

  it(
    "Track F3's Check: editing text BEFORE a PII span does not misplace its highlight",
    () => {
      const text = "Contact Jean Dupont at work.";
      const finding = piiEntity({ category: "person", start: 8, end: 19 }); // "Jean Dupont"
      const { state, field } = makeState(text, [finding]);
      expect(ranges(state, field)).toEqual([{ from: 8, to: 19, category: "person" }]);

      // Insert "Please " (7 chars) at the very start of the document — entirely before the
      // decorated span.
      const tr = state.update({ changes: { from: 0, insert: "Please " } });
      const next = tr.state;

      const mapped = ranges(next, field);
      expect(mapped).toEqual([{ from: 15, to: 26, category: "person" }]); // shifted by 7
      // The decoration still bounds exactly the original PII text, not a misaligned slice.
      expect(next.doc.sliceString(mapped[0].from, mapped[0].to)).toBe("Jean Dupont");
    },
  );

  it("editing text AFTER a PII span leaves its highlight untouched", () => {
    const text = "Contact Jean Dupont at work.";
    const finding = piiEntity({ category: "person", start: 8, end: 19 });
    const { state, field } = makeState(text, [finding]);

    const tr = state.update({ changes: { from: text.length, insert: " Thanks!" } });
    const mapped = ranges(tr.state, field);
    expect(mapped).toEqual([{ from: 8, to: 19, category: "person" }]);
  });

  it("editing text INSIDE a PII span shrinks the decoration to what survives, not to nothing", () => {
    // CM6's default mapping (associativity aside) keeps a mark decoration around
    // surviving text when an edit lands inside its range, rather than deleting the whole
    // decoration — documenting that behavior here so a future change notices if it drifts.
    const text = "Contact Jean Dupont at work.";
    const finding = piiEntity({ category: "person", start: 8, end: 19 }); // "Jean Dupont"
    const { state, field } = makeState(text, [finding]);

    // Delete "Jean " (5 chars) from inside the span.
    const tr = state.update({ changes: { from: 8, to: 13 } });
    const mapped = ranges(tr.state, field);
    expect(mapped).toEqual([{ from: 8, to: 14, category: "person" }]);
    expect(tr.state.doc.sliceString(8, 14)).toBe("Dupont");
  });
});
