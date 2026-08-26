import * as React from "react";
import { FileText, Folder } from "lucide-react";
import { toast } from "sonner";
import { Button } from "@/components/ui/button";
import { traverseDataTransferItems } from "@/lib/folder-upload";

export interface DropzoneProps {
  readonly onFiles: (files: File[]) => void;
  readonly maxFiles?: number;
  readonly maxSize?: number;
  readonly folderMode?: boolean;
  readonly onToggleFolderMode?: (event: React.MouseEvent) => void;
}

const ACCEPT = ".pdf,.docx,.xlsx,.pptx,.jpg,.png,.txt,.json,.fec,.csv";

export function Dropzone({
  onFiles,
  maxFiles = 50,
  maxSize: _maxSize = 50 * 1024 * 1024,
  folderMode = false,
  onToggleFolderMode,
}: DropzoneProps) {
  const [dragOver, setDragOver] = React.useState(false);
  const inputRef = React.useRef<HTMLInputElement>(null);

  React.useEffect(() => {
    const input = inputRef.current;
    if (input === null) return;
    input.webkitdirectory = folderMode;
    if (folderMode) {
      input.setAttribute("webkitdirectory", "");
    } else {
      input.removeAttribute("webkitdirectory");
    }
  }, [folderMode]);

  function handleClick(): void {
    inputRef.current?.click();
  }

  function handleKeyDown(event: React.KeyboardEvent<HTMLDivElement>): void {
    if (event.key === "Enter" || event.key === " ") {
      event.preventDefault();
      handleClick();
    }
  }

  function emit(files: File[]): void {
    if (files.length === 0) return;
    if (files.length > maxFiles) {
      toast.error(`Limite de ${String(maxFiles)} documents dépassée`);
      onFiles(files.slice(0, maxFiles));
      return;
    }
    onFiles(files);
  }

  function onDragOver(event: React.DragEvent<HTMLDivElement>): void {
    event.preventDefault();
    setDragOver(true);
  }

  function onDragLeave(): void {
    setDragOver(false);
  }

  async function onDrop(event: React.DragEvent<HTMLDivElement>): Promise<void> {
    event.preventDefault();
    setDragOver(false);
    const items = Array.from(event.dataTransfer.items);
    let files: File[] = [];
    let containsDirectory = false;
    for (const item of items) {
      const getter = item.webkitGetAsEntry;
      if (typeof getter === "function") {
        const entry = getter.call(item);
        if (entry !== null && entry.isDirectory) {
          containsDirectory = true;
          break;
        }
      }
    }
    if (containsDirectory && items.length > 0) {
      files = await traverseDataTransferItems(items);
    } else {
      files = Array.from(event.dataTransfer.files);
    }
    emit(files);
  }

  function onChange(event: React.ChangeEvent<HTMLInputElement>): void {
    const list = event.target.files;
    if (list === null) return;
    const files = Array.from(list);
    emit(files);
    // Reset value so selecting the same file again still fires change
    event.target.value = "";
  }

  return (
    <div
      role="button"
      tabIndex={0}
      aria-label="Déposer des documents"
      onClick={handleClick}
      onKeyDown={handleKeyDown}
      onDragOver={onDragOver}
      onDragLeave={onDragLeave}
      onDrop={onDrop}
      className={`flex flex-col items-center justify-center rounded-xl border-2 border-dashed p-12 text-center transition-colors ${
        dragOver ? "border-violet-400 bg-violet-50" : "border-slate-200 bg-white"
      }`}
    >
      <div className="mb-6 flex items-center justify-center">
        {/* 4 overlapping doc icons */}
        <div className="flex -space-x-3">
          <div className="flex h-12 w-10 items-center justify-center rounded-md border border-slate-200 bg-blue-50 shadow-sm">
            <FileText size={20} className="text-blue-500" />
          </div>
          <div className="flex h-12 w-10 items-center justify-center rounded-md border border-slate-200 bg-red-50 shadow-sm">
            <FileText size={20} className="text-red-500" />
          </div>
          <div className="flex h-12 w-10 items-center justify-center rounded-md border border-slate-200 bg-orange-50 shadow-sm">
            <FileText size={20} className="text-orange-500" />
          </div>
          <div className="flex h-12 w-10 items-center justify-center rounded-md border border-slate-200 bg-green-50 shadow-sm">
            <FileText size={20} className="text-green-600" />
          </div>
        </div>
      </div>

      <p className="text-sm font-medium text-slate-900">Cliquez ou glissez-déposez vos documents</p>
      <p className="mt-1 text-xs text-slate-500">
        50 documents maximum · 50 Mo chacun · PDF,DOCX,XLSX,PPTX,images et texte(TXT,JSON,FEC etc.)
      </p>

      {onToggleFolderMode !== undefined ? (
        <div className="mt-4 flex items-center gap-2">
          <Button
            type="button"
            variant={folderMode ? "outline" : "secondary"}
            size="sm"
            onClick={(event: React.MouseEvent) => {
              event.stopPropagation();
              onToggleFolderMode(event);
            }}
            aria-pressed={folderMode === false}
          >
            <FileText size={14} className="mr-1" /> Fichier
          </Button>
          <Button
            type="button"
            variant={folderMode ? "secondary" : "outline"}
            size="sm"
            onClick={(event: React.MouseEvent) => {
              event.stopPropagation();
              onToggleFolderMode(event);
            }}
            aria-pressed={folderMode}
          >
            <Folder size={14} className="mr-1" /> Dossier
          </Button>
        </div>
      ) : null}

      <input
        ref={inputRef}
        type="file"
        multiple
        accept={ACCEPT}
        className="hidden"
        onChange={onChange}
        tabIndex={-1}
      />
    </div>
  );
}
