/** Read-only, line-numbered text panel — the Redacted/Source/Layout/Audit tabs in the
 * document detail view all render pre-computed text, not something a user edits, so a
 * plain scrollable `<pre>` with a gutter is enough; the editable case (manual PII
 * tagging) still goes through `MarkdownEditor`, not this. */
export function CodeLines({ text }: { text: string }) {
  const lines = text.split("\n");
  return (
    <div className="flex h-full overflow-auto font-mono text-xs leading-relaxed">
      <div className="select-none border-r border-border px-3 py-2 text-right text-muted-foreground">
        {lines.map((_, i) => (
          <div key={i}>{i + 1}</div>
        ))}
      </div>
      <pre className="flex-1 whitespace-pre-wrap break-words px-3 py-2">{text}</pre>
    </div>
  );
}
