import * as React from "react";
import { Card, CardHeader, CardTitle, CardContent } from "@/components/ui/card";
import { Button } from "@/components/ui/button";
import { Upload } from "lucide-react";

export interface DropzoneProps {
  onFiles: (files: File[]) => void;
}

export function Dropzone({ onFiles }: DropzoneProps) {
  const onDrop = (e: React.DragEvent) => {
    e.preventDefault();
    const files = Array.from(e.dataTransfer.files);
    onFiles(files);
  };

  return (
    <Card>
      <CardHeader>
        <CardTitle className="text-sm">Déposez vos fichiers</CardTitle>
      </CardHeader>
      <CardContent>
        <div onDragOver={e=>e.preventDefault()} onDrop={onDrop} className="rounded-lg border border-dashed p-8 text-center">
          <Upload className="mx-auto mb-2 h-6 w-6 text-muted-foreground" />
          <p className="text-xs text-muted-foreground mb-3">Glissez-déposez ici ou</p>
          <Button size="sm" variant="outline">Parcourir</Button>
        </div>
      </CardContent>
    </Card>
  );
}
