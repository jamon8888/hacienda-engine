import * as React from "react";
import { ResizablePanelGroup, ResizablePanel, ResizableHandle } from "@/components/ui/resizable";
import { CategorySidebar } from "./CategorySidebar";
import type { PiiEntity } from "@/lib/pii-engine";

export interface EditorLayoutProps {
  readonly findings: ReadonlyArray<PiiEntity>;
  readonly onRemoveFinding: (index: number) => void;
  readonly onAddFinding: (start: number, end: number, category: string) => void;
  readonly documentText: string;
  readonly redactedText: string;
}

export function EditorLayout({ findings, onRemoveFinding, onAddFinding, documentText, redactedText }: EditorLayoutProps) {
  return (
    <ResizablePanelGroup orientation="horizontal" className="flex h-full min-h-[600px]">
      <ResizablePanel defaultSize={20} minSize={15} maxSize={35}>
        <div className="h-full border-r">
          <CategorySidebar findings={findings} onRemove={onRemoveFinding} />
        </div>
      </ResizablePanel>
      <ResizableHandle withHandle />
      <ResizablePanel defaultSize={50} minSize={30}>
        <div className="h-full overflow-auto p-4">
          <h2 className="mb-2 text-sm font-medium">Document original</h2>
          <pre className="whitespace-pre-wrap text-xs">{documentText}</pre>
        </div>
      </ResizablePanel>
      <ResizableHandle withHandle />
      <ResizablePanel defaultSize={30} minSize={20}>
        <div className="h-full overflow-auto p-4">
          <h2 className="mb-2 text-sm font-medium">Occurrences</h2>
          <p className="text-xs text-muted-foreground">Redacted: {redactedText.slice(0, 200)}</p>
        </div>
      </ResizablePanel>
    </ResizablePanelGroup>
  );
}
