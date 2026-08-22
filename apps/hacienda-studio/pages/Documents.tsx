import { useEffect, useMemo, useState } from "react";
import { Archive, Upload, FileText, Download, FolderDown, FileDown, Folder, Eye, Trash2 } from "lucide-react";
import { toast } from "sonner";
import { Button } from "@/components/ui/button";
import { FileSystem } from "@/components/extend/file-system";
import type { FileSystemFileItem, FileSystemItem } from "@/components/extend/file-system";
import { DocxViewerPreview } from "@/components/extend/docx-viewer";
import { XlsxViewerPreview } from "@/components/extend/xlsx-viewer";
import { PptxViewerPreview } from "@/components/extend/pptx-viewer";
import { PDFViewer } from "@/components/extend/pdf-viewer";
import { toFileSystemItems, type LibraryDocument } from "@/lib/library";
import { getViewerKind } from "@/lib/viewer-kind";
import { effectiveFileName } from "@/lib/file-filter";
import { exportDocumentsZip } from "@/lib/export-zip";

function downloadMarkdown(name: string, markdown: string) {
  const blob = new Blob([markdown], { type: "text/markdown" });
  const url = URL.createObjectURL(blob);
  const a = document.createElement("a");
  a.href = url;
  a.download = name.endsWith(".md") ? name : `${name}.md`;
  a.click();
  URL.revokeObjectURL(url);
}

/**
 * Redesign: the Documents page is now `components/extend/file-system.tsx`'s
 * macOS-Finder-style browser. Finder is centered on screen (max-w + mx-auto) so
 * folder/file cards sit in the visual middle rather than hugging the left edge —
 * scanning stays comfortable at any viewport width.
 *
 * Downloads integrate at three calibrated points — no extra chrome:
 * 1) Header "Export all" (zip of every redacted markdown) — primary bulk action
 * 2) Centered selection bar (appears on single select) — file → Download .md + Open; folder → Download folder .zip
 * 3) File open always goes to detail → there you see extracted markdown with PII chips
 */
export function Documents({
  documents,
  files,
  onOpenDocument,
  onAddFiles,
  onDeleteDocuments,
}: {
  documents: LibraryDocument[];
  files: File[];
  onOpenDocument: (name: string) => void;
  onAddFiles: () => void;
  onDeleteDocuments?: (names: string[]) => void;
}) {
  const items = useMemo(() => toFileSystemItems(documents), [documents]);
  const byPath = useMemo(
    () => new Map(documents.map((d) => [d.result.name, d] as const)),
    [documents],
  );

  const [selected, setSelected] = useState<FileSystemItem | null>(null);

  // keep selection valid when library changes (e.g. delete in detail)
  useEffect(() => {
    if (!selected) return;
    const stillExists =
      selected.kind === "file"
        ? byPath.has(selected.path)
        : documents.some((d) => d.result.name.startsWith(selected.path));
    if (!stillExists) setSelected(null);
  }, [byPath, documents, selected]);

  function findOriginalFile(item: FileSystemFileItem): File | undefined {
    const doc = byPath.get(item.path);
    if (!doc) return undefined;
    return files.find((f) => effectiveFileName(f) === doc.result.frontmatter.source);
  }

  const selectedFileDoc = selected?.kind === "file" ? byPath.get(selected.path) ?? null : null;
  const selectedFolderDocs =
    selected?.kind === "folder"
      ? documents.filter((d) => d.result.name.startsWith(selected.path))
      : [];

  function handleDownloadFile() {
    if (!selectedFileDoc) return;
    downloadMarkdown(selectedFileDoc.result.name, selectedFileDoc.result.markdown);
  }
  function handleDownloadFolder() {
    if (selected?.kind !== "folder" || selectedFolderDocs.length === 0) return;
    const folderName = selected.path.replace(/\/$/, "").split("/").pop() || "folder";
    exportDocumentsZip(
      selectedFolderDocs.map((d) => ({ result: d.result })),
      `hacienda-${folderName}-${selectedFolderDocs.length}.zip`,
    );
  }
  function handleDownloadAll() {
    exportDocumentsZip(documents.map((d) => ({ result: d.result })));
  }
  function handleDeleteFile() {
    if (!selectedFileDoc || !onDeleteDocuments) return;
    onDeleteDocuments([selectedFileDoc.result.name]);
    toast.success(`Deleted ${selectedFileDoc.result.frontmatter.source}`);
    setSelected(null);
  }
  function handleDeleteFolder() {
    if (selected?.kind !== "folder" || selectedFolderDocs.length === 0 || !onDeleteDocuments) return;
    const names = selectedFolderDocs.map((d) => d.result.name);
    onDeleteDocuments(names);
    const folderName = selected.path.replace(/\/$/, "").split("/").pop() ?? "folder";
    toast.success(`Deleted “${folderName}” — ${names.length} file${names.length === 1 ? "" : "s"}`);
    setSelected(null);
  }

  return (
    <div className="flex flex-1 flex-col bg-[#070a10] px-4 py-6 sm:px-6">
      {/* Centered page shell */}
      <div className="mx-auto flex w-full max-w-[1100px] flex-1 flex-col">
        <div className="mb-4 flex flex-col gap-3">
          <div className="flex flex-col gap-3 lg:grid lg:grid-cols-[1fr_auto_1fr] lg:items-center">
            <div className="min-w-0">
              <h1 className="text-xl font-semibold tracking-tight">Documents</h1>
              <p className="mt-1 truncate text-xs text-white/50">
                {documents.length} processed · stored locally on this device
              </p>
            </div>

            {/* Center — per-file / per-folder redacted download, responsive */}
            <div className="flex justify-start lg:justify-center">
              {selected ? (
                <div className="inline-flex max-w-full items-center gap-2 rounded-full border border-white/[0.08] bg-[#151a24] px-2 py-1.5 shadow-[0_4px_24px_rgba(0,0,0,0.35)]">
                  <span className="hidden size-7 items-center justify-center rounded-full bg-white/[0.06] text-white/60 sm:flex">
                    {selected.kind === "folder" ? <Folder className="size-3.5" /> : <FileText className="size-3.5" />}
                  </span>
                  <span className="max-w-[18ch] truncate font-mono text-xs font-medium text-white/90 sm:max-w-[24ch]">
                    {selected.kind === "folder"
                      ? (selected.path.replace(/\/$/, "").split("/").pop() ?? "folder") + "/"
                      : (selected as FileSystemFileItem).name ?? selected.path}
                  </span>
                  <span className="hidden text-[11px] text-white/30 sm:inline">
                    {selected.kind === "folder"
                      ? `${selectedFolderDocs.length} files`
                      : selectedFileDoc
                        ? `${selectedFileDoc.findings.length} findings`
                        : ""}
                  </span>
                  <span className="h-5 w-px bg-white/[0.08]" aria-hidden />
                  {selected.kind === "file" && selectedFileDoc ? (
                    <>
                      <Button
                        size="sm"
                        variant="ghost"
                        className="h-7 rounded-full px-3 text-xs text-white/70 hover:bg-white/[0.06] hover:text-white"
                        onClick={() => onOpenDocument(selected.path)}
                      >
                        <Eye className="size-3.5" /> Open
                      </Button>
                      <Button
                        size="sm"
                        className="h-7 rounded-full bg-white px-3.5 text-xs font-medium text-[#0a0e13] hover:bg-white/90"
                        onClick={handleDownloadFile}
                      >
                        <FileDown className="size-3.5" /> Download
                      </Button>
                      <span className="mx-1 h-5 w-px bg-white/[0.08]" aria-hidden />
                      <button
                        type="button"
                        aria-label="Delete file"
                        onClick={handleDeleteFile}
                        className="flex size-7 items-center justify-center rounded-full bg-white/[0.04] text-white/40 hover:bg-destructive/15 hover:text-destructive"
                        title="Delete file"
                      >
                        <Trash2 className="size-3.5" />
                      </button>
                    </>
                  ) : selected.kind === "folder" ? (
                    <>
                      <Button
                        size="sm"
                        className="h-7 rounded-full bg-white px-3.5 text-xs font-medium text-[#0a0e13] hover:bg-white/90 disabled:opacity-40"
                        disabled={selectedFolderDocs.length === 0}
                        onClick={handleDownloadFolder}
                      >
                        <FolderDown className="size-3.5" /> Download folder
                      </Button>
                      <span className="mx-1 h-5 w-px bg-white/[0.08]" aria-hidden />
                      <button
                        type="button"
                        aria-label="Delete folder"
                        onClick={handleDeleteFolder}
                        className="flex size-7 items-center justify-center rounded-full bg-white/[0.04] text-white/40 hover:bg-destructive/15 hover:text-destructive"
                        title={`Delete folder and ${selectedFolderDocs.length} files`}
                      >
                        <Trash2 className="size-3.5" />
                      </button>
                    </>
                  ) : null}
                  <button
                    type="button"
                    aria-label="Clear selection"
                    onClick={() => setSelected(null)}
                    className="ml-1 flex size-6 items-center justify-center rounded-full text-white/30 hover:bg-white/[0.06] hover:text-white/60"
                  >
                    ×
                  </button>
                </div>
              ) : (
                <span className="hidden text-xs text-white/20 lg:inline" aria-hidden>
                  Select a file or folder to download
                </span>
              )}
            </div>

            <div className="flex items-center gap-2 lg:justify-end">
              <Button variant="outline" size="sm" onClick={onAddFiles} className="h-8 border-white/10 bg-white/[0.04] hover:bg-white/[0.08]">
                <Upload className="size-3.5" /> Add files
              </Button>
              <Button
                size="sm"
                className="h-8 bg-white text-[#0a0e13] hover:bg-white/90"
                disabled={documents.length === 0}
                onClick={handleDownloadAll}
              >
                <Archive className="size-3.5" /> Export all
              </Button>
            </div>
          </div>
          {/* Mobile: show selection hint below header when selected */}
          {selected && (
            <p className="text-center text-[11px] text-white/30 lg:hidden">
              {selected.kind === "file"
                ? "Selected file · redacted .md ready"
                : `Selected folder · ${selectedFolderDocs.length} redacted files`}
            </p>
          )}
        </div>

        {/* Centered finder — single border (FileSystem owns its rounded-xl border), no double wrapper */}
        <div className="flex min-h-[520px] flex-1 flex-col">
          {documents.length === 0 ? (
            <div className="grid flex-1 place-items-center rounded-xl border border-white/[0.06] bg-[#0f1419] p-10 text-center shadow-[0_8px_40px_rgba(0,0,0,0.35)]">
              <div className="max-w-sm">
                <div className="mx-auto flex size-10 items-center justify-center rounded-xl border border-white/[0.06] bg-white/[0.04]">
                  <Folder className="size-5 text-white/40" />
                </div>
                <p className="mt-3 text-sm font-medium">No documents yet</p>
                <p className="mt-1 text-xs leading-relaxed text-white/40">
                  Add files to run the on-device pipeline. Source previews for <span className="font-mono text-[11px]">pdf</span>, <span className="font-mono text-[11px]">docx</span>, <span className="font-mono text-[11px]">pptx</span>, <span className="font-mono text-[11px]">xlsx</span> render here; click a file to see the extracted markdown with PII chips.
                </p>
                <Button size="sm" className="mt-4" onClick={onAddFiles}>
                  <Upload className="size-3.5" /> Add files
                </Button>
              </div>
            </div>
          ) : (
            <FileSystem
              items={items}
              title="Documents"
              defaultView="icons"
              onFileOpen={(file) => onOpenDocument(file.path)}
              onSelectionChange={(item) => setSelected(item)}
              renderFilePreview={(file) => (
                <DocumentThumbnail file={file} originalFile={findOriginalFile(file)} />
              )}
            />
          )}
        </div>

        <p className="mx-auto mt-3 max-w-[640px] text-center text-[11px] leading-relaxed text-white/30">
          Click a file to open its extracted markdown with PII chips. Select a file or folder to download the redacted output — single <span className="font-mono">.md</span> for a file, <span className="font-mono">.zip</span> of all <span className="font-mono">documents/*.md</span> for a folder.
        </p>
      </div>
    </div>
  );
}

/**
 * Grid-cell preview: a live, scaled-down instance of the same native viewer
 * `DocumentDetail`'s Source tab uses, for a file type that has one and whose original
 * bytes are still in memory this session. Everything else gets a generic icon — "for
 * type that can", per the redesign brief, not a fabricated thumbnail.
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

  return (
    <div className="relative h-full w-full overflow-hidden bg-background">
      <div
        className="pointer-events-none absolute left-0 top-0 origin-top-left"
        style={{ width: "400%", height: "400%", transform: "scale(0.25)" }}
      >
        {(viewerKind === "docx" || viewerKind === "doc") && (
          <DocxViewerPreview src={url} fileName={file.name ?? file.path} isDark showUpload={false} showToolbar={false} onIsDarkChange={() => {}} />
        )}
        {(viewerKind === "xlsx" || viewerKind === "xls") && (
          <XlsxViewerPreview src={url} fileName={file.name ?? file.path} isDark showUpload={false} showToolbar={false} onIsDarkChange={() => {}} />
        )}
        {(viewerKind === "pptx" || viewerKind === "ppt") && (
          <PptxViewerPreview src={url} fileName={file.name ?? file.path} showUpload={false} showToolbar={false} />
        )}
        {viewerKind === "pdf" && <PDFViewer src={url} fileName={file.name ?? file.path} showUpload={false} showToolbar={false} />}
      </div>
    </div>
  );
}
