import * as React from "react";
import { Card, CardContent } from "@/components/ui/card";
import { Textarea } from "@/components/ui/textarea";
import { CloudUpload, FileText, Type } from "lucide-react";

export function RestorationView() {
  return (
    <div className="space-y-4">
      <p className="text-xs text-muted-foreground">Importez des fichiers contenant des pseudonymes pour restaurer leurs valeurs d'origine.</p>
      <Card>
        <CardContent className="pt-6 space-y-6">
          <div className="grid grid-cols-2 gap-4">
            <div className="rounded-lg border border-dashed border-primary/40 p-10 flex flex-col items-center justify-center text-center gap-2 min-h-[200px]">
              <CloudUpload className="size-8 text-primary/60" />
              <p className="text-xs font-medium">Cliquez ou déposez des fichiers à restaurer</p>
              <p className="text-[10px] text-muted-foreground">Formats acceptés : TXT, MD, DOCX, XLSX, PPTX</p>
            </div>
            <div className="flex flex-col items-center justify-center text-center gap-2 min-h-[200px]">
              <FileText className="size-8 text-muted-foreground/40" />
              <p className="text-xs text-muted-foreground">Les fichiers restaurés apparaîtront ici</p>
            </div>
          </div>
          <div className="grid grid-cols-2 gap-4">
            <div className="space-y-2">
              <p className="text-xs text-muted-foreground">Collez du texte contenant des pseudonymes pour restaurer leurs valeurs d'origine</p>
              <Textarea placeholder="Collez du texte à restaurer..." className="min-h-[160px]" />
            </div>
            <div className="flex flex-col items-center justify-center text-center gap-2 min-h-[160px]">
              <Type className="size-8 text-muted-foreground/40" />
              <p className="text-xs text-muted-foreground">Le texte restauré apparaîtra ici</p>
            </div>
          </div>
        </CardContent>
      </Card>
    </div>
  );
}
