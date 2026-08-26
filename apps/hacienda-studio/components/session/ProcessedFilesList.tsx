import * as React from "react";
import { Card, CardHeader, CardTitle, CardContent } from "@/components/ui/card";
import { Button } from "@/components/ui/button";
import { Trash2, Eye } from "lucide-react";

export interface ProcessedFileItem {
  name: string;
  size: number;
}

export interface ProcessedFilesListProps {
  files: ReadonlyArray<ProcessedFileItem>;
  onView?: (name: string) => void;
  onDelete?: (name: string) => void;
}

export function ProcessedFilesList({ files, onView, onDelete }: ProcessedFilesListProps) {
  return (
    <Card>
      <CardHeader>
        <CardTitle className="text-sm">Fichiers traités</CardTitle>
      </CardHeader>
      <CardContent>
        <ul className="space-y-2">
          {files.map(f => (
            <li key={f.name} className="flex items-center justify-between rounded border p-2">
              <span className="text-xs">{f.name}</span>
              <div className="flex gap-1">
                <Button size="icon" variant="ghost" onClick={()=>onView?.(f.name)}><Eye className="h-3 w-3" /></Button>
                <Button size="icon" variant="ghost" onClick={()=>onDelete?.(f.name)}><Trash2 className="h-3 w-3" /></Button>
              </div>
            </li>
          ))}
          {files.length===0 && <p className="text-xs text-muted-foreground">Aucun fichier traité</p>}
        </ul>
      </CardContent>
    </Card>
  );
}
