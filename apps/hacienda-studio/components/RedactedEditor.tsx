/**
 * Track K2: the side-by-side split view's right pane — a free-text-editable redacted
 * markdown buffer, seeded once from `renderAnnotatedMarkdown` but independent of it from
 * that point on. Unlike `MarkdownEditor` (uncontrolled after mount, decorations mapped
 * through edits by CM6 itself), this is a controlled editor: `value` is owned by `App.tsx`
 * so the "Redact selection" button can splice a category token into the buffer via
 * `onChange`, the same way a paste or keystroke would.
 */
import { useCallback, useMemo, useState } from "react";
import CodeMirror from "@uiw/react-codemirror";
import type { ViewUpdate } from "@uiw/react-codemirror";
import { markdown } from "@codemirror/lang-markdown";
import { oneDark } from "@codemirror/theme-one-dark";
import {
  ADDABLE_CATEGORIES,
  DEFAULT_ADDABLE_CATEGORY,
  isAddableCategory,
} from "../lib/pii-categories";
import { spliceRedaction } from "../lib/annotate";

// See MarkdownEditor.tsx's identical constant for why this is module-level, not inline.
const BASIC_SETUP = { lineNumbers: false, foldGutter: false };
// Redesign: the app is dark-only (see app.css's `.dark` block) — CM6 defaults to a light
// theme regardless of the page around it, so without this the editor was the one visibly
// light panel in an otherwise dark UI.
const EXTENSIONS = [markdown(), oneDark];

export function RedactedEditor({
  value,
  onChange,
}: {
  value: string;
  onChange: (next: string) => void;
}) {
  const [selection, setSelection] = useState({ from: 0, to: 0 });
  const [category, setCategory] = useState(DEFAULT_ADDABLE_CATEGORY);

  const handleUpdate = useCallback((update: ViewUpdate) => {
    const range = update.state.selection.main;
    setSelection({ from: range.from, to: range.to });
  }, []);

  const extensions = useMemo(() => EXTENSIONS, []);

  return (
    <div className="flex flex-col gap-2">
      <div className="flex items-center gap-2 text-sm">
        <select
          className="redact-category rounded-md border border-border bg-background px-2 py-1 text-xs"
          value={category}
          onChange={(e) => {
            if (isAddableCategory(e.target.value)) setCategory(e.target.value);
          }}
        >
          {ADDABLE_CATEGORIES.map((c) => (
            <option key={c} value={c}>
              {c}
            </option>
          ))}
        </select>
        <button
          type="button"
          className="redact-selection rounded-md bg-muted px-3 py-1 text-xs text-foreground transition-colors hover:bg-primary hover:text-primary-foreground disabled:cursor-not-allowed disabled:opacity-50"
          disabled={selection.from === selection.to}
          onClick={() => {
            const next = spliceRedaction(value, selection.from, selection.to, category);
            onChange(next);
            setSelection({ from: selection.from, to: selection.from });
          }}
        >
          Redact selection
        </button>
      </div>
      <CodeMirror
        value={value}
        extensions={extensions}
        basicSetup={BASIC_SETUP}
        className="cm-redacted-editor overflow-hidden rounded-md border border-border text-sm"
        onChange={onChange}
        onUpdate={handleUpdate}
      />
    </div>
  );
}
