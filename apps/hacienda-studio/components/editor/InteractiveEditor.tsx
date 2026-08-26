import * as React from "react";
import { Button } from "@/components/ui/button";
import { MarkdownEditor } from "@/components/MarkdownEditor";
import { AddDetectionDialog } from "./AddDetectionDialog";
import type { PiiEntity } from "@/lib/pii-engine";

export interface InteractiveEditorProps {
  value: string;
  findings: ReadonlyArray<PiiEntity>;
  onAddFinding: (start: number, end: number, category: string) => void;
}

export function InteractiveEditor({ value, findings, onAddFinding }: InteractiveEditorProps) {
  const [dialogOpen, setDialogOpen] = React.useState(false);
  const [selection, setSelection] = React.useState<{ start: number; end: number } | null>(null);

  return (
    <div className="space-y-2">
      <div className="flex gap-2">
        <Button size="sm" variant="outline" onClick={()=>setDialogOpen(true)}>Sélectionner</Button>
        <Button size="sm" variant="outline">Retirer</Button>
        <Button size="sm" variant="outline">Extraire vers le haut</Button>
      </div>
      <MarkdownEditor value={value} findings={findings as PiiEntity[]} onAddFinding={(s,e,c)=>onAddFinding(s,e,c)} />
      <AddDetectionDialog open={dialogOpen} onOpenChange={setDialogOpen} onAdd={(s,e,c)=>onAddFinding(s,e,c)} />
    </div>
  );
}
