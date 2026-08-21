import { useEffect, useMemo, useState } from "react";
import { Archive, Upload, FileText } from "lucide-react";
import { Button } from "@/components/ui/button";
import { FileSystem } from "@/components/extend/file-system";
import type { FileSystemFileItem } from "@/components/extend/file-system";
import { DocxViewerPreview } from "@/components/extend/docx-viewer";
import { XlsxViewerPreview } from "@/components/extend/xlsx-viewer";
import { PptxViewerPreview } from "@/components/extend/pptx-viewer";
import { PDFViewer } from "@/components/extend/pdf-viewer";
import { toFileSystemItems, type LibraryDocument } from "@/lib/library";
import { getViewerKind } from "@/lib/viewer-kind";
import { effectiveFileName } from "@/lib/file-filter";
import { exportDocumentsZip } from "@/lib/export-zip";

/**
 * Redesign: the Documents page is now `components/extend/file-system.tsx`'s
 * macOS-Finder-style browser (icon/list/gallery views, folders inferred from path,
 * search, sort — all built in) instead of a hand-rolled table. Folders fall out of
 * `LibraryDocument.result.name`'s slashes for free; `lib/library.ts`'s
 * `toFileSystemItems` is the only mapping needed.
 *
 * `FileSystem`'s selection model is single-item (`onSelectionChange?: (item) => void`,
 * no multi-select array), so the old checkbox-based bulk export/delete doesn't have an
 * equivalent here. Export now always zips everything currently in the library; deleting
 * a single document moved to `DocumentDetail`'s header instead of a grid bulk action.
 */
export function Documents({
  documents,
  files,
  onOpenDocument,
  onAddFiles,
}: {
  documents: LibraryDocument[];
  /** This session's in-memory originals — only files processed in this tab session
   * have bytes here (see `lib/persistence.ts`'s header); a hydrated-from-storage
   * document has none, so its grid thumbnail falls back to a generic icon. */
  files: File[];
  onOpenDocument: (name: string) => void;
  onAddFiles: () => void;
}) {
  const items = useMemo(() => toFileSystemItems(documents), [documents]);
  const byPath = useMemo(
    () => new Map(documents.map((d) => [d.result.name, d] as const)),
    [documents],
  );

  function findOriginalFile(item: FileSystemFileItem): File | undefined {
    const doc = byPath.get(item.path);
    if (!doc) return undefined;
    return files.find((f) => effectiveFileName(f) === doc.result.frontmatter.source);
  }

  return (
    <div className="flex flex-1 flex-col px-6 py-8">
      <div className="mb-4 flex items-center justify-between">
        <div>
          <h1 className="text-2xl font-semibold">Documents</h1>
          <p className="text-sm text-muted-foreground">
            {documents.length} processed · stored locally on this device
          </p>
        </div>
        <div className="flex items-center gap-2">
          <Button variant="outline" onClick={onAddFiles}>
            <Upload className="size-4" /> Add files
          </Button>
          <Button
            disabled={documents.length === 0}
            onClick={() => exportDocumentsZip(documents.map((d) => ({ result: d.result })))}
          >
            <Archive className="size-4" /> Export
          </Button>
        </div>
      </div>

      <div className="min-h-0 flex-1 overflow-hidden rounded-lg border border-border">
        {documents.length === 0 ? (
          <p className="p-8 text-center text-sm text-muted-foreground">No documents yet.</p>
        ) : (
          <FileSystem
            items={items}
            title="Documents"
            defaultView="icons"
            onFileOpen={(file) => onOpenDocument(file.path)}
            renderFilePreview={(file) => (
              <DocumentThumbnail file={file} originalFile={findOriginalFile(file)} />
            )}
          />
        )}
      </div>
    </div>
  );
}

/**
 * Grid-cell preview: a live, scaled-down instance of the same native viewer
 * `DocumentDetail`'s Source tab uses, for a file type that has one and whose original
 * bytes are still in memory this session. Everything else gets a generic icon — "for
 * type that can", per the redesign brief, not a fabricated thumbnail.
 *
 * Known tradeoff: this mounts a real docx/xlsx/pptx/pdf parser per visible grid cell.
 * Fine for a modest library; a very large one would want virtualization or a lazy
 * generated-thumbnail pipeline instead, neither of which this on-device app has.
 */
function DocumentThumbnail({
  file,
  originalFile,
}: {
  file: FileSystemFileItem;
  originalFile: File | undefined;
}) {
  const [url, setUrl] = useState<string | undefined>(undefined);
  const isImage = file.contentType?.startsWith("image/") ?? false;
  const viewerKind = getViewerKind(file.name ?? file.path);

  useEffect(() => {
    if (!originalFile) {
      setUrl(undefined);
      return;
    }
    const objectUrl = URL.createObjectURL(originalFile);
    setUrl(objectUrl);
    return () => URL.revokeObjectURL(objectUrl);
  }, [originalFile]);

  if (!url || (!isImage && !viewerKind)) {
    return (
      <div className="flex h-full w-full flex-col items-center justify-center gap-2 bg-muted/30">
        <FileText className="size-8 text-muted-foreground" />
        <span className="font-mono text-[0.6875rem] uppercase tracking-wide text-muted-foreground">
          {(file.name ?? file.path).split(".").pop()}
        </span>
      </div>
    );
  }

  if (isImage) {
    return (
      <img
        src={url}
        alt={file.name ?? file.path}
        className="h-full w-full object-cover"
      />
    );
  }

  // Renders at 4x size then scales down 4x — a cheap way to get a readable-looking
  // thumbnail from a viewer component built for full-size display, without a second
  // "compact mode" rendering path for each of these four viewers.
  return (
    <div className="relative h-full w-full overflow-hidden bg-background">
      <div
        className="pointer-events-none absolute left-0 top-0 origin-top-left"
        style={{ width: "400%", height: "400%", transform: "scale(0.25)" }}
      >
        {(viewerKind === "docx" || viewerKind === "doc") && (
          <DocxViewerPreview src={url} isDark showUpload={false} showToolbar={false} onIsDarkChange={() => {}} />
        )}
        {(viewerKind === "xlsx" || viewerKind === "xls") && (
          <XlsxViewerPreview src={url} isDark showUpload={false} showToolbar={false} onIsDarkChange={() => {}} />
        )}
        {(viewerKind === "pptx" || viewerKind === "ppt") && (
          <PptxViewerPreview src={url} showUpload={false} showToolbar={false} />
        )}
        {viewerKind === "pdf" && <PDFViewer src={url} showUpload={false} showToolbar={false} />}
      </div>
    </div>
  );
}
