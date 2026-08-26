import * as React from "react";
import { Card, CardContent } from "@/components/ui/card";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { X, Check, MoreHorizontal } from "lucide-react";

export type ProcessedFile = { id: string; name: string; status: string; };

export function ProcessedFilesList({ files }: { files: ReadonlyArray<ProcessedFile> }) {
  return (
    <div className="space-y-2">
      <h3 className="text-sm font-medium">Fichiers traités</h3>
      <div className="space-y-2">
        {files.map(f => (
          <Card key={f.id}>
            <CardContent className="flex items-center justify-between py-2">
              <div className="flex items-center gap-2 min-w-0">
                <Badge variant="secondary" className="text-[10px]">{f.status}</Badge>
                <span className="truncate text-xs">{f.name}</span>
              </div>
              <div className="flex items-center gap-1">
                <Button variant="ghost" size="icon" className="h-7 w-7">
                  <Check className="size-3" />
                </Button>
                <Button variant="ghost" size="icon" className="h-7 w-7">
                  <MoreHorizontal className="size-3" />
                </Button>
              </div>
            </CardContent>
          </Card>
        ))}
      </div>
    </div>
  );
}
