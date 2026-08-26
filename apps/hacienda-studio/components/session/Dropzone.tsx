import * as React from "react";
import { Card, CardContent } from "@/components/ui/card";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Upload, Files, Paperclip } from "lucide-react";

export function Dropzone({
  onDropFiles,
  onDropFolder,
  disabled,
}: {
  onDropFiles?: (files: FileList) => void;
  onDropFolder?: (files: FileList) => void;
  disabled?: boolean;
}) {
  const [drag, setDrag] = React.useState(false);
  const inputRef = React.useRef<HTMLInputElement>(null);

  return (
    <div
      onDragOver={e => { e.preventDefault(); setDrag(true); }}
      onDragLeave={() => setDrag(false)}
      onDrop={e => {
        e.preventDefault();
        setDrag(false);
        const items = e.dataTransfer.files;
        onDropFiles?.(items);
      }}
      className={`flex items-center justify-center rounded-xl border-2 ${drag ? "border-primary" : "border-muted-foreground/25"} bg-muted/50 min-h-[220px]`}
    >
      <div className="text-center space-y-4">
        <Upload className="mx-auto size-12 text-muted-foreground/50" />
        <div className="space-y-1">
          <p className="font-medium">Cliquez pour téléverser ou <span className="text-primary underline cursor-pointer" onClick={()=>inputRef.current?.click()}>ouvrez la sélection de fichiers</span></p>
          <p className="text-xs text-muted-foreground">Formats supportés : PDF, DOCX, DOC, XLSX, XLS, PPTX, PPT, LV …</p>
        </div>
        <div className="flex items-center justify-center gap-2">
          <Button variant="outline" size="sm" onClick={()=>inputRef.current?.click()} disabled={disabled}>Choisir le fichier</Button>
          <Button variant="outline" size="sm" disabled={disabled}>Choisir un dossier</Button>
        </div>
        <input ref={inputRef} type="file" multiple className="hidden" />
      </div>
    </div>
  );
}
