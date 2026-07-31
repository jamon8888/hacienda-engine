/**
 * Track F3: CodeMirror 6 decorations marking PII spans inline, built to survive document
 * edits — the plan's own Check ("editing text before a PII span does not misplace that
 * span's highlight"). CM6 decorations are a `RangeSet` carried in a `StateField`, mapped
 * through every transaction's `changes` (`value.map(tr.changes)`); that mapping is exactly
 * what keeps a decoration anchored to the same underlying text after an edit earlier in the
 * document shifts its offset — the same problem Track F4's `renderAnnotatedMarkdown` solved
 * for the export path, solved here for the live editor by a mechanism CM6 provides rather
 * than one this codebase has to reimplement.
 *
 * Deliberately does not embed a reveal UI inside the editor: doing that means a CM6 widget
 * decoration hosting a React portal (Popover, passphrase input, the works) inside a
 * non-React DOM tree — real engineering, not a small addition, and not what this Check
 * asks for. The editor's job here is showing *where* PII is in the live text; revealing it
 * stays in `components/PiiPanel.tsx`'s list, right next to the editor.
 */
import { RangeSetBuilder, StateField, type Extension } from "@codemirror/state";
import { Decoration, EditorView, type DecorationSet } from "@codemirror/view";
import type { PiiEntity } from "./pii-engine";

/**
 * Builds the field's initial `DecorationSet` from `findings`' `start`/`end` offsets against
 * `doc`. Spans are sorted and any that fall outside `doc`'s bounds (a finding computed
 * against different text than what's loaded into the editor) are dropped rather than
 * throwing — `RangeSetBuilder` requires strictly ascending, in-bounds ranges.
 */
export function buildDecorations(findings: PiiEntity[], docLength: number): DecorationSet {
  const builder = new RangeSetBuilder<Decoration>();
  const sorted = [...findings]
    .filter((f) => f.start >= 0 && f.end <= docLength && f.start < f.end)
    .sort((a, b) => a.start - b.start || a.end - b.end);

  let lastEnd = -1;
  for (const finding of sorted) {
    // A RangeSetBuilder rejects out-of-order/overlapping ranges; two findings whose spans
    // overlap (already possible upstream — see Track F4's own overlap handling) would
    // otherwise crash the whole editor instead of just skipping the second decoration.
    if (finding.start < lastEnd) continue;
    builder.add(
      finding.start,
      finding.end,
      Decoration.mark({
        class: "cm-pii-pill",
        attributes: { "data-pii-category": finding.category },
      }),
    );
    lastEnd = finding.end;
  }
  return builder.finish();
}

/** A `StateField` extension for `@uiw/react-codemirror`'s `extensions` prop. */
export function piiHighlightExtension(findings: PiiEntity[]): Extension {
  const field = StateField.define<DecorationSet>({
    create(state) {
      return buildDecorations(findings, state.doc.length);
    },
    update(decorations, tr) {
      // The mapping step: re-anchors every existing decoration to its new offset after
      // `tr.changes`, without recomputing from `findings` — an edit doesn't change *which*
      // text is PII, only where it now sits.
      return decorations.map(tr.changes);
    },
  });
  return [field, EditorView.decorations.from(field)];
}
