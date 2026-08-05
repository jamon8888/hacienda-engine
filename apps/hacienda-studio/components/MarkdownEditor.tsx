/**
 * Track F3: CodeMirror 6 markdown editor with inline PII decorations. Nothing existed to
 * build on — CodeMirror was not a dependency in any repo, and hacienda-private's document
 * pane is a read-only `<pre>` of already-redacted text rendered with `react-markdown`, not
 * an editor. See `lib/pii-decorations.ts` for the decoration extension itself (offset
 * mapping through edits, the plan's stated Check) — this component only wires it into
 * `@uiw/react-codemirror` and the app's Tailwind theme.
 *
 * Track I4: `onAddFinding`, when given, turns on a "Mark selection as PII" toolbar — the
 * add half of add/remove editing (removal lives in `PiiPanel`). The selected range is read
 * straight off CM6's own `EditorState.selection`, which is already in the *editor's*
 * coordinates — the same `rawMarkdown` offsets `findings[].start/.end` use, since this
 * component is uncontrolled after mount and never re-splices the document CM6 holds. No
 * separate offset translation is needed for that reason.
 *
 * Known scope cut: `findings` changing identity (an add or remove in `App.tsx`) makes
 * `@uiw/react-codemirror` reconfigure this component's extensions, which rebuilds the
 * decoration `StateField` from scratch via `buildDecorations` — a fresh `create()`, not a
 * continuation of F3's `decorations.map(tr.changes)` history. Fine when the findings array
 * changes before or after the user has typed into the editor, not interleaved with it: an
 * edit made between two add/remove actions would have the *pre-existing* findings'
 * decorations rebuilt from their original offsets rather than their edit-shifted positions.
 * Solving that would mean routing add/remove through CM6 `StateEffect`s dispatched directly
 * at the live view instead of a prop change — real engineering, out of scope here the same
 * way embedding a reveal UI inside the editor was ruled out above.
 *
 * `extensions` and `onUpdate` below are both memoized to stable identities (on `findings`,
 * and permanently, respectively). `@uiw/react-codemirror`'s reconfigure effect fires
 * whenever either prop's *identity* changes, not just `findings`' contents — tracking
 * selection via `onUpdate` fires on every keystroke, and an inline arrow function there
 * would reconfigure (and, per the paragraph above, silently corrupt) the decoration field
 * on every keystroke of ordinary typing, not just on an actual I4 edit. This is exactly the
 * F3 Check's failure mode, so it is not a hypothetical the tests skip.
 */
import { useCallback, useMemo, useRef, useState } from "react";
import CodeMirror from "@uiw/react-codemirror";
import type { ReactCodeMirrorRef, ViewUpdate } from "@uiw/react-codemirror";
import { markdown } from "@codemirror/lang-markdown";
import type { PiiEntity } from "../lib/pii-engine";
import { piiHighlightExtension } from "../lib/pii-decorations";

const ADDABLE_CATEGORIES = [
  "person",
  "email",
  "phone",
  "address",
  "organization",
  "other",
];

// Module-level, not inline in the JSX below: `@uiw/react-codemirror`'s internal
// reconfigure effect depends on this prop's *identity* (like `extensions` per the
// class comment above), and its `onUpdate` fires on that reconfigure dispatch too —
// an inline `{ ... }` literal here is a fresh reference every render, so every
// `onUpdate`-driven re-render would reconfigure again, which fires `onUpdate` again,
// forever. A stable reference is required, not just a memoized one, since this value
// never changes.
const BASIC_SETUP = { lineNumbers: false, foldGutter: false };

export function MarkdownEditor({
  value,
  findings,
  onAddFinding,
}: {
  value: string;
  findings: PiiEntity[];
  onAddFinding?: (start: number, end: number, category: string) => void;
}) {
  // Uncontrolled after mount on purpose: the PII decorations are computed once from the
  // findings' original offsets and then mapped through every subsequent edit by CM6 itself
  // (see lib/pii-decorations.ts). Feeding a changed `value` back in as a controlled prop
  // would replace the whole document and, with it, the very decorations this component
  // exists to keep correctly anchored.
  const [content] = useState(value);
  const [selection, setSelection] = useState({ from: 0, to: 0 });
  const [category, setCategory] = useState(ADDABLE_CATEGORIES[0]);
  const editorRef = useRef<ReactCodeMirrorRef>(null);

  const extensions = useMemo(
    () => [markdown(), piiHighlightExtension(findings)],
    [findings],
  );
  const handleUpdate = useCallback((update: ViewUpdate) => {
    const range = update.state.selection.main;
    setSelection({ from: range.from, to: range.to });
  }, []);

  return (
    <div className="flex flex-col gap-2">
      {onAddFinding && (
        <div className="flex items-center gap-2 text-sm">
          <select
            className="pii-add-category rounded-md border border-border bg-background px-2 py-1 text-xs"
            value={category}
            onChange={(e) => setCategory(e.target.value)}
          >
            {ADDABLE_CATEGORIES.map((c) => (
              <option key={c} value={c}>
                {c}
              </option>
            ))}
          </select>
          <button
            type="button"
            className="pii-mark-selection rounded-md bg-muted px-3 py-1 text-xs text-foreground transition-colors hover:bg-primary hover:text-primary-foreground disabled:cursor-not-allowed disabled:opacity-50"
            disabled={selection.from === selection.to}
            onClick={() => {
              onAddFinding(selection.from, selection.to, category);
            }}
          >
            Mark selection as PII
          </button>
        </div>
      )}
      <CodeMirror
        ref={editorRef}
        value={content}
        extensions={extensions}
        basicSetup={BASIC_SETUP}
        className="cm-pii-editor overflow-hidden rounded-md border border-border text-sm"
        onUpdate={onAddFinding ? handleUpdate : undefined}
      />
    </div>
  );
}
