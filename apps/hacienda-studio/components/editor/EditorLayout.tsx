import * as React from "react";
import { PanelGroup, Panel, PanelResizeHandle } from "@/components/ui/resizable";
import { ScrollArea } from "@/components/ui/scroll-area";
import { CategorySidebar } from "@/components/editor/CategorySidebar";
import { InteractiveEditor } from "@/components/editor/InteractiveEditor";
import { PseudonymGrid } from "@/components/pseudonyms/PseudonymGrid";

export interface EditorLayoutProps {
  category: "INPUT" | "RED" | "EXT" | "CPR" | "GLN";
  onCategoryChange?: (c: string) => void;
}

export function EditorLayout({ category, onCategoryChange }: EditorLayoutProps) {
  return (
    <PanelGroup direction="horizontal" className="h-full w-full">
      <Panel defaultSize={20} minSize={12} maxSize={34}>
        <ScrollArea className="h-full">
          <CategorySidebar activeGroup={category} onChange={onCategoryChange} />
        </ScrollArea>
      </Panel>
      <PanelResizeHandle />
      <Panel defaultSize={40} minSize={25}>
        <ScrollArea className="h-full p-4">
          <InteractiveEditor />
        </ScrollArea>
      </Panel>
      <PanelResizeHandle />
      <Panel defaultSize={40} minSize={25}>
        <ScrollArea className="h-full p-4">
          <PseudonymGrid groups={[]} />
        </ScrollArea>
      </Panel>
    </PanelGroup>
  );
}
