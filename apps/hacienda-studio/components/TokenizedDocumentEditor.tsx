import { useCallback, useState } from "react";
import { EditorContent, useEditor } from "@tiptap/react";
import { Mark, mergeAttributes } from "@tiptap/core";
import StarterKit from "@tiptap/starter-kit";
import {
  applyManualRedaction,
  type SensitiveMode,
  type TokenizedEditorDocument,
} from "@/lib/tokenized-editor";

const HaciendaSensitive = Mark.create({
  name: "haciendaSensitive",
  inclusive: false,
  addAttributes() {
    return {
      tokenClass: { default: "sensitive" },
      tokenId: { default: "manual" },
      mode: { default: "manual-mask" },
      source: { default: "manual" },
      disclosure: { default: "viewer-only" },
    };
  },
  parseHTML() {
    return [{ tag: "span[data-hacienda-sensitive]" }];
  },
  renderHTML({ HTMLAttributes }) {
    return [
      "span",
      mergeAttributes(HTMLAttributes, {
        "data-hacienda-sensitive": "true",
        class: "hacienda-sensitive-token",
      }),
      0,
    ];
  },
});

export function TokenizedDocumentEditor({
  value,
  onChange,
}: {
  value: TokenizedEditorDocument;
  onChange: (next: TokenizedEditorDocument) => void;
}) {
  const [category, setCategory] = useState("person");
  const [selection, setSelection] = useState({ from: 0, to: 0 });
  const editor = useEditor({
    extensions: [StarterKit, HaciendaSensitive],
    content: value,
    editorProps: { attributes: { class: "tokenized-tiptap-editor" } },
    onSelectionUpdate({ editor: nextEditor }) {
      setSelection({
        from: nextEditor.state.selection.from,
        to: nextEditor.state.selection.to,
      });
    },
    onUpdate({ editor: nextEditor }) {
      onChange(nextEditor.getJSON() as unknown as TokenizedEditorDocument);
    },
  });

  const redactSelection = useCallback(
    (mode: SensitiveMode) => {
      if (!editor || selection.from === selection.to) return;
      const selectedText = editor.state.doc.textBetween(
        selection.from,
        selection.to,
        "\n",
      );
      const from = editor.state.doc.textBetween(0, selection.from, "\n").length;
      const to = from + selectedText.length;
      const next = applyManualRedaction({
        document: editor.getJSON() as unknown as TokenizedEditorDocument,
        from,
        to,
        category,
        mode,
      });
      editor.commands.setContent(next as never);
      onChange(next);
    },
    [category, editor, onChange, selection],
  );

  if (!editor)
    return (
      <div className="rounded-lg border border-border p-4 text-sm text-muted-foreground">
        Chargement de l’éditeur tokenisé…
      </div>
    );

  return (
    <div className="flex flex-col gap-2">
      <div className="flex flex-wrap items-center gap-2 text-sm">
        <select
          aria-label="Sensitive category"
          className="rounded-md border border-border bg-background px-2 py-1 text-xs"
          value={category}
          onChange={(event) => setCategory(event.target.value)}
        >
          <option value="person">person</option>
          <option value="organization">organization</option>
          <option value="email">email</option>
          <option value="phone">phone</option>
          <option value="matter">matter</option>
        </select>
        <button
          type="button"
          className="rounded-md bg-muted px-3 py-1 text-xs hover:bg-primary hover:text-primary-foreground disabled:opacity-50"
          disabled={selection.from === selection.to}
          onClick={() => redactSelection("pseudonymize")}
        >
          Anonymiser la sélection
        </button>
        <button
          type="button"
          className="rounded-md bg-muted px-3 py-1 text-xs hover:bg-primary hover:text-primary-foreground disabled:opacity-50"
          disabled={selection.from === selection.to}
          onClick={() => redactSelection("manual-mask")}
        >
          Masquer la sélection
        </button>
      </div>
      <EditorContent editor={editor} />
    </div>
  );
}
