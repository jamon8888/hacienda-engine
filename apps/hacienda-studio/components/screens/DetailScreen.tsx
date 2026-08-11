/**
 * Screen 5 of 5: a single result's detail view, reached by clicking a `FileBrowser` row.
 * Replicates the two rendering branches `App.tsx` used to pick between inline via
 * `isOpen`/`viewerKind`/`previewUrl` (see git history) — split view (native viewer left,
 * a `Tabs`-switched "Redacted Markdown"/"Findings" pane right) when a native viewer exists
 * for this file type, findings-only fallback (`MarkdownEditor` + `PiiPanel`, Track I4)
 * otherwise. Every result is openable now (see `FileBrowser.tsx`'s `openable` comment),
 * not just viewer-having ones, so this screen must not assume `viewerKind`/`previewUrl`
 * are present.
 *
 * Track K/Phase 3: the split view's right pane used to be `RedactedEditor` alone, with
 * Track I4's findings UI only reachable when there was no viewer at all. Folding both into
 * one `Tabs` component (plan `enchanted-seeking-wozniak.md` §5) means a docx/pdf/etc. file
 * can use both editing surfaces without a third resizable panel. "Redacted Markdown" stays
 * the default tab (`defaultValue`, not `forceMount`) — `split-view.spec.ts` asserts
 * `.cm-redacted-editor` is visible immediately after opening, with no tab click first.
 */
import { ArrowLeftIcon } from "lucide-react";
import { Button } from "../ui/button";
import { ResizableHandle, ResizablePanel, ResizablePanelGroup } from "../ui/resizable";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "../ui/tabs";
import { DocxViewerPreview } from "../extend/docx-viewer";
import { XlsxViewerPreview } from "../extend/xlsx-viewer";
import { PptxViewerPreview } from "../extend/pptx-viewer";
import { PDFViewer } from "../extend/pdf-viewer";
import { RedactedEditor } from "../RedactedEditor";
import { MarkdownEditor } from "../MarkdownEditor";
import { PiiPanel } from "../PiiPanel";
import type { ProcessedFile } from "../../lib/types";
import type { ViewerKind } from "../../lib/viewer-kind";
import type { PiiEntity } from "../../lib/pii-engine";

export function DetailScreen({
  result,
  viewerKind,
  previewUrl,
  viewerDark,
  onViewerDarkChange,
  redactedValue,
  onRedactedChange,
  saveStatus,
  findings,
  edited,
  onAddFinding,
  onRemoveFinding,
  onExportEdited,
  onDownloadZip,
  onBack,
}: {
  result: ProcessedFile;
  viewerKind: ViewerKind;
  previewUrl: string | undefined;
  viewerDark: boolean;
  onViewerDarkChange: (dark: boolean) => void;
  redactedValue: string;
  onRedactedChange: (next: string) => void;
  saveStatus: "saving" | "saved" | null;
  findings: PiiEntity[];
  edited: boolean;
  onAddFinding: (start: number, end: number, category: string) => void;
  onRemoveFinding: (index: number) => void;
  onExportEdited: () => void;
  onDownloadZip: () => void;
  onBack: () => void;
}) {
  const hasViewer = !!(viewerKind && previewUrl);

  const findingsPane = (
    <div className="flex flex-col gap-2">
      <MarkdownEditor value={result.rawMarkdown} findings={findings} onAddFinding={onAddFinding} />
      {(findings.length > 0 || edited) && <PiiPanel findings={findings} onRemove={onRemoveFinding} />}
      {edited && (
        <button
          type="button"
          className="export-edited self-start rounded-md bg-muted px-2 py-1 text-xs text-foreground hover:bg-primary hover:text-primary-foreground"
          onClick={onExportEdited}
        >
          Export edited file
        </button>
      )}
    </div>
  );

  return (
    <section aria-label="File detail" className="mx-auto w-full max-w-[1100px] flex-1 px-6 py-12">
      <div className="flex items-center justify-between text-sm">
        <Button variant="ghost" size="sm" className="close-split-view" onClick={onBack}>
          <ArrowLeftIcon /> Back
        </Button>
        <span className="flex items-center gap-2 font-medium">
          {result.name}
          {hasViewer && saveStatus && (
            <span className="save-status text-xs font-normal text-muted-foreground">
              {saveStatus === "saving" ? "Saving…" : "Saved"}
            </span>
          )}
        </span>
        <div className="flex items-center gap-2">
          {!hasViewer && (
            <span className="text-muted-foreground">
              {result.entities.length} {result.entities.length === 1 ? "entity" : "entities"}
            </span>
          )}
          <button
            type="button"
            className="download-zip rounded-md bg-primary px-3 py-1.5 text-xs text-primary-foreground hover:bg-primary/90"
            onClick={onDownloadZip}
          >
            Download zip
          </button>
        </div>
      </div>

      {hasViewer ? (
        <ResizablePanelGroup
          orientation="horizontal"
          className="split-view mt-4 h-[70vh] rounded-lg border"
        >
          {/* `defaultSize`/`minSize` are percentages here: bare numbers are pixels in this
              library's API, strings-without-units are percent (see react-resizable-panels'
              own doc comments) — pass strings to get a 50/50 split, not a 50px one. */}
          <ResizablePanel defaultSize="50" minSize="25" className="min-w-0 overflow-auto p-2">
            {viewerKind === "docx" && (
              <DocxViewerPreview
                src={previewUrl}
                fileName={result.name}
                isDark={viewerDark}
                onIsDarkChange={onViewerDarkChange}
                showUpload={false}
              />
            )}
            {viewerKind === "doc" && (
              <DocxViewerPreview
                src={previewUrl}
                fileName={result.name}
                isDark={viewerDark}
                onIsDarkChange={onViewerDarkChange}
                showUpload={false}
              />
            )}
            {viewerKind === "xlsx" && (
              <XlsxViewerPreview
                src={previewUrl}
                fileName={result.name}
                isDark={viewerDark}
                onIsDarkChange={onViewerDarkChange}
                showUpload={false}
              />
            )}
            {viewerKind === "xls" && (
              <XlsxViewerPreview
                src={previewUrl}
                fileName={result.name}
                isDark={viewerDark}
                onIsDarkChange={onViewerDarkChange}
                showUpload={false}
              />
            )}
            {viewerKind === "pptx" && (
              <PptxViewerPreview src={previewUrl} fileName={result.name} showUpload={false} />
            )}
            {viewerKind === "ppt" && (
              <PptxViewerPreview src={previewUrl} fileName={result.name} showUpload={false} />
            )}
            {viewerKind === "pdf" && <PDFViewer src={previewUrl} fileName={result.name} showUpload={false} />}
          </ResizablePanel>
          <ResizableHandle withHandle />
          <ResizablePanel defaultSize="50" minSize="25" className="min-w-0 overflow-auto p-2">
            <Tabs defaultValue="redacted" className="h-full">
              <TabsList variant="line">
                <TabsTrigger value="redacted">Redacted Markdown</TabsTrigger>
                <TabsTrigger value="findings">Findings</TabsTrigger>
              </TabsList>
              <TabsContent value="redacted">
                <RedactedEditor value={redactedValue} onChange={onRedactedChange} />
              </TabsContent>
              <TabsContent value="findings">{findingsPane}</TabsContent>
            </Tabs>
          </ResizablePanel>
        </ResizablePanelGroup>
      ) : (
        <div className="mt-4">{findingsPane}</div>
      )}
    </section>
  );
}
