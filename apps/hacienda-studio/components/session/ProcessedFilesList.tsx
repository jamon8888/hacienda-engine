import * as React from "react";
import { FileText, Trash2 } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Card } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";

export interface ProcessedFileItem {
  readonly name: string;
  readonly subtitle: string;
  readonly pseudonymized?: boolean;
}

export interface ProcessedFilesListProps {
  readonly files: ReadonlyArray<ProcessedFileItem>;
  readonly onView: (name: string) => void;
  readonly onDelete: (name: string) => void;
  readonly onAddFiles?: () => void;
}

function fileIconColor(name: string): string {
  if (name.endsWith(".pdf.md") || name.includes(".pdf")) return "bg-red-50 text-red-500 border-red-200";
  if (name.endsWith(".txt")) return "bg-slate-50 text-slate-500 border-slate-200";
  return "bg-green-50 text-green-600 border-green-200";
}

export function ProcessedFilesList({ files, onView, onDelete, onAddFiles }: ProcessedFilesListProps) {
  return (
    <div className="flex flex-col gap-4">
      {onAddFiles ? (
        <Card className="border border-dashed border-slate-200 bg-white p-4">
          <div className="flex items-center justify-between">
            <div>
              <p className="text-sm font-medium text-slate-900">Ajouter des documents</p>
              <p className="text-xs text-slate-500">Cliquez ou glissez-déposez des fichiers</p>
            </div>
            <Button variant="outline" size="sm" onClick={onAddFiles}>
              Parcourir
            </Button>
          </div>
          <p className="mt-1 text-xs text-slate-400">Formats supportés : PDF, DOCX, XLSX, PPTX, images et textes (TXT, JSON, FEC, etc.)</p>
        </Card>
      ) : null}

      <div className="flex items-center justify-between">
        <h2 className="text-sm font-semibold text-slate-900">Fichiers traités</h2>
        <span className="text-xs text-slate-500">{files.length} fichier{files.length > 1 ? "s" : ""}</span>
      </div>

      <div className="flex flex-col gap-3">
        {files.map((file) => (
          <Card
            key={file.name}
            className="flex items-center justify-between border border-green-200 bg-green-50/50 p-3"
          >
            <div className="flex min-w-0 items-center gap-3">
              <div className={`flex h-9 w-9 shrink-0 items-center justify-center rounded-md border bg-white ${fileIconColor(file.name)}`}>
                <FileText size={18} />
              </div>
              <div className="min-w-0">
                <p className="truncate text-sm font-medium text-slate-900">{file.name}</p>
                <p className="text-xs text-slate-500">{file.subtitle}</p>
              </div>
              {file.pseudonymized ? (
                <Badge variant="outline" className="hidden bg-white text-xs text-green-700 sm:inline-flex">
                  P
                </Badge>
              ) : null}
            </div>
            <div className="ml-3 flex shrink-0 items-center gap-2">
              <Button
                size="sm"
                className="bg-violet-500 text-white hover:bg-violet-600"
                onClick={() => onView(file.name)}
              >
                Afficher
              </Button>
              <Button
                variant="ghost"
                size="icon"
                className="h-8 w-8 text-red-400 hover:bg-red-50 hover:text-red-500"
                aria-label={`Supprimer ${file.name}`}
                onClick={() => onDelete(file.name)}
              >
                <Trash2 size={16} />
              </Button>
            </div>
          </Card>
        ))}
      </div>
    </div>
  );
}
