/**
 * Track F3: CodeMirror 6 markdown editor with inline PII decorations. Nothing existed to
 * build on — CodeMirror was not a dependency in any repo, and hacienda-private's document
 * pane is a read-only `<pre>` of already-redacted text rendered with `react-markdown`, not
 * an editor. See `lib/pii-decorations.ts` for the decoration extension itself (offset
 * mapping through edits, the plan's stated Check) — this component only wires it into
 * `@uiw/react-codemirror` and the app's Tailwind theme.
 */
import { useState } from "react";
import CodeMirror from "@uiw/react-codemirror";
import { markdown } from "@codemirror/lang-markdown";
import type { PiiEntity } from "../lib/pii-engine";
import { piiHighlightExtension } from "../lib/pii-decorations";

export function MarkdownEditor({
  value,
  findings,
}: {
  value: string;
  findings: PiiEntity[];
}) {
  // Uncontrolled after mount on purpose: the PII decorations are computed once from the
  // findings' original offsets and then mapped through every subsequent edit by CM6 itself
  // (see lib/pii-decorations.ts). Feeding a changed `value` back in as a controlled prop
  // would replace the whole document and, with it, the very decorations this component
  // exists to keep correctly anchored.
  const [content] = useState(value);

  return (
    <CodeMirror
      value={content}
      extensions={[markdown(), piiHighlightExtension(findings)]}
      basicSetup={{ lineNumbers: false, foldGutter: false }}
      className="cm-pii-editor overflow-hidden rounded-md border border-border text-sm"
    />
  );
}
