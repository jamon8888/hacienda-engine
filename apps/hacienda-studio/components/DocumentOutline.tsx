import { useMemo, useState, type RefObject } from "react";
import { ChevronDown, ChevronRight, List } from "lucide-react";
import { extractHeadings } from "@/lib/document-outline";
import type { PiiEntity } from "@/lib/pii-engine";

/**
 * Scrolls the first text node inside `container` whose content includes `headingText`
 * into view. Every left-panel renderer (`CodeLines`, `PiiAnnotatedView`,
 * `MarkdownEditor`/CodeMirror) shows `rawMarkdown` verbatim rather than parsing it, so
 * the heading's `#`-prefixed line is present as literal text in all of them — a DOM text
 * search works across every renderer without any of them needing a scroll-to-offset prop.
 */
function scrollToHeading(container: HTMLElement, headingText: string): void {
  const walker = document.createTreeWalker(container, NodeFilter.SHOW_TEXT);
  let node: Node | null;
  while ((node = walker.nextNode())) {
    if (node.textContent?.includes(headingText)) {
      const target = node.parentElement;
      target?.scrollIntoView({ block: "center", behavior: "smooth" });
      return;
    }
  }
}

export function DocumentOutline({
  markdown,
  findings,
  containerRef,
}: {
  markdown: string;
  findings: PiiEntity[];
  containerRef: RefObject<HTMLElement | null>;
}) {
  const [collapsed, setCollapsed] = useState(false);
  const headings = useMemo(() => extractHeadings(markdown, findings), [markdown, findings]);

  if (headings.length === 0) return null;

  return (
    <div className="shrink-0 border-b border-border bg-[#0d1218]">
      <button
        type="button"
        onClick={() => setCollapsed((c) => !c)}
        className="flex w-full items-center gap-1.5 px-4 py-2 text-[11px] font-medium uppercase tracking-widest text-muted-foreground hover:text-foreground"
        aria-expanded={!collapsed}
      >
        {collapsed ? <ChevronRight className="size-3.5" /> : <ChevronDown className="size-3.5" />}
        <List className="size-3.5" />
        Outline
        <span className="ml-auto font-mono text-[10px] normal-case tracking-normal text-muted-foreground/70">
          {headings.length} section{headings.length === 1 ? "" : "s"}
        </span>
      </button>
      {!collapsed && (
        <nav className="max-h-48 overflow-auto px-2 pb-2">
          {headings.map((heading, i) => (
            <button
              key={`${heading.start}-${i}`}
              type="button"
              onClick={() => {
                const container = containerRef.current;
                if (container) scrollToHeading(container, heading.text);
              }}
              className="flex w-full items-center gap-2 rounded px-2 py-1 text-left text-xs text-foreground/80 hover:bg-muted"
              style={{ paddingLeft: `${8 + (heading.level - 1) * 12}px` }}
            >
              <span className="truncate">{heading.text}</span>
              {heading.piiCount > 0 && (
                <span className="ml-auto inline-flex shrink-0 items-center gap-1 rounded-full border border-amber-500/20 bg-amber-500/10 px-1.5 py-0.5 font-mono text-[10px] normal-case tracking-normal text-amber-300">
                  <span className="size-1 rounded-full bg-amber-400" />
                  {heading.piiCount}
                </span>
              )}
            </button>
          ))}
        </nav>
      )}
    </div>
  );
}
