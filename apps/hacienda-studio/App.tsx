import { useEffect, useRef, useState } from "react";
import { Onboarding } from "./components/Onboarding";
import { ConfigPanel } from "./components/ConfigPanel";
import { ProgressBar } from "./components/ProgressBar";
import { PiiPanel } from "./components/PiiPanel";
import { MarkdownEditor } from "./components/MarkdownEditor";
import { RedactedEditor } from "./components/RedactedEditor";
import { FileBrowser, buildFileRows } from "./components/FileBrowser";
import { FileUpload } from "./components/extend/file-upload";
import { DocxViewerPreview } from "./components/extend/docx-viewer";
import { XlsxViewerPreview } from "./components/extend/xlsx-viewer";
import { PptxViewerPreview } from "./components/extend/pptx-viewer";
import { PDFViewer } from "./components/extend/pdf-viewer";
import { loadNerModel, isModelCached, preloadXbergWasm, validateFile } from "./lib/asset-loader";
import { effectiveFileName, isJunkFile } from "./lib/file-filter";
import { renderAnnotatedMarkdown } from "./lib/annotate";
import { getViewerKind } from "./lib/viewer-kind";
import { DEFAULT_CONFIG } from "./lib/types";
import type { AppConfig, OnboardingState, ProcessedFile, ProgressUpdate } from "./lib/types";
import type { PiiEntity } from "./lib/pii-engine";

const UPLOAD_ACCEPT =
  ".pdf,.docx,.xlsx,.pptx,.odt,.ods,.odp,.eml,.msg,.pst,.png,.jpg,.jpeg,.gif,.webp,.tiff,.bmp,.svg,.srt,.vtt,.txt,.md,.json,.csv,.xml,.html,.mp3,.wav,.m4a,.ogg,.flac,.aac,.mp4,.mov,.webm,.mkv";

function downloadZip(blob: Blob): void {
  const url = URL.createObjectURL(blob);
  const a = document.createElement("a");
  a.href = url;
  a.download = `xberg-output-${Date.now()}.zip`;
  a.click();
  URL.revokeObjectURL(url);
}

function downloadText(fileName: string, content: string): void {
  const blob = new Blob([content], { type: "text/markdown" });
  const url = URL.createObjectURL(blob);
  const a = document.createElement("a");
  a.href = url;
  a.download = fileName;
  a.click();
  URL.revokeObjectURL(url);
}

/**
 * Track I4: re-splices `result.rawMarkdown` against the (possibly edited) `findings`,
 * reusing the exact function `worker/pipeline.ts` used to build the original export —
 * `lib/annotate.ts` exists specifically so this can happen on the main thread without
 * importing the worker module itself (see that file's header). Frontmatter and the local
 * "## Entities" glossary are carried over unchanged from the original export rather than
 * regenerated: `pii_entities_found` and the entity list can go stale relative to add/remove
 * edits (an added finding won't retroactively drop an entity mention from the glossary, a
 * removed one won't add a mention back), which is an explicit, documented scope cut — full
 * frontmatter/registry/KG-export consistency with post-export edits is out of scope for
 * this increment, the same kind of bounded call already made for I3 below and for Track
 * J2's "thinner CLI vault" decision.
 */
function reExportMarkdown(result: ProcessedFile, findings: PiiEntity[]): string {
  const docPath = "documents/" + result.name;
  const linked = renderAnnotatedMarkdown(result.rawMarkdown, result.entities, findings, docPath);
  const frontmatterMatch = result.markdown.match(/^---\n[\s\S]*?\n---/);
  const glossaryMatch = result.markdown.match(/\n## Entities\n\n[\s\S]*$/);
  const frontmatter = frontmatterMatch ? frontmatterMatch[0] : "";
  const glossary = glossaryMatch ? glossaryMatch[0] : "";
  return frontmatter + "\n" + linked + glossary;
}

export function App() {
  const [onboardingComplete, setOnboardingComplete] = useState(false);
  const [assets, setAssets] = useState<OnboardingState["assets"]>({
    xbergWasm: false,
    nerModel: false,
    tessdata: false,
  });
  const [files, setFiles] = useState<File[]>([]);
  const [progress, setProgress] = useState<Map<string, ProgressUpdate>>(new Map());
  const [results, setResults] = useState<ProcessedFile[]>([]);
  const [config, setConfig] = useState<AppConfig>({ ...DEFAULT_CONFIG });
  const [showConfig, setShowConfig] = useState(false);
  const [error, setError] = useState<string | null>(null);
  // Toggles the *same* `#file-input` element between file- and directory-picking rather
  // than adding a second `<input type="file">` — every e2e test's `input[type="file"]`
  // selector assumes there is exactly one, and a second element would make it ambiguous.
  const [folderMode, setFolderMode] = useState(false);
  // Folder drops routinely carry OS noise and unsupported files; report the count once
  // instead of a per-file error banner that just overwrites itself.
  const [skipNotice, setSkipNotice] = useState<string | null>(null);
  // `assets.nerModel` stays true even when the neural backend failed to load, because the
  // app has a legitimate regex-only fallback and onboarding must not get stuck on a
  // blocked model download. This flag is what actually records the failure.
  const [nerModelDegraded, setNerModelDegraded] = useState(false);
  // The drop zone renders before the worker finishes its handshake, and the handshake is
  // slow — it compiles a 48 MB WASM module. Dropping a file into that window used to throw
  // on a null worker and silently do nothing.
  const [workerReady, setWorkerReady] = useState(false);
  // Track I3: per-file failures, keyed by the same `effectiveFileName` used for `progress`
  // — the global `error` banner above still fires too (existing UX contract), this is what
  // lets the Finder-like list show *which* file failed instead of only a banner that
  // overwrites itself once per failure.
  const [fileErrors, setFileErrors] = useState<Map<string, string>>(new Map());
  // Track I4: edits layered on top of a result's original `piiFindings`, keyed by
  // `ProcessedFile.name` (the output name, unique per batch — unlike the input name, which
  // a folder upload can repeat across sibling directories). Absent from this map means
  // "unedited, use `result.piiFindings` as-is"; present (even as an unchanged copy) is what
  // the Finder list's "edited" badge keys off, via `handleAddFinding`/`handleRemoveFinding`
  // always writing an entry, never mutating `result.piiFindings` itself.
  const [editedFindings, setEditedFindings] = useState<Map<string, PiiEntity[]>>(new Map());
  // Keyed by `ProcessedFile.name` (the `.md` output name), object URLs backing the
  // docx/xlsx/pptx/pdf preview viewers below. `results` is never cleared mid-session
  // (see the batch-complete handler above), so these are revoked only on unmount.
  const [previewUrls, setPreviewUrls] = useState<Map<string, string>>(new Map());
  // A single viewer-scoped dark toggle — this app has no app-wide dark mode.
  const [viewerDark, setViewerDark] = useState(false);
  // Track K1: which result (if any) is expanded into the side-by-side split view, opened
  // via a FileBrowser row click. Only one at a time — the split view is a focused reading
  // mode, not another always-stacked list.
  const [openResultName, setOpenResultName] = useState<string | null>(null);
  // Track K2: the redacted pane's free-text buffer per result, keyed by `ProcessedFile.name`.
  // Seeded lazily from `renderAnnotatedMarkdown` the first time a result's split view opens
  // (see `redactedDraftFor` below); once present, edits here are independent of
  // `piiFindings`/`entities` offsets, same scope cut as `MarkdownEditor`'s own edit model.
  const [redactedDrafts, setRedactedDrafts] = useState<Map<string, string>>(new Map());

  const workerRef = useRef<Worker | null>(null);
  const configRef = useRef(config);
  configRef.current = config;
  const previewUrlsRef = useRef(previewUrls);
  previewUrlsRef.current = previewUrls;

  useEffect(() => {
    setPreviewUrls((prev) => {
      const next = new Map(prev);
      let changed = false;
      for (const result of results) {
        if (next.has(result.name)) continue;
        const file = files.find(
          (f) => effectiveFileName(f) === result.frontmatter.source,
        );
        if (!file) continue;
        next.set(result.name, URL.createObjectURL(file));
        changed = true;
      }
      return changed ? next : prev;
    });
  }, [results, files]);

  useEffect(() => {
    // Reads the ref (not `previewUrls`) so the map at the moment of unmount is
    // revoked, not the empty map this effect's own closure was created with.
    return () => {
      for (const url of previewUrlsRef.current.values()) URL.revokeObjectURL(url);
    };
  }, []);

  useEffect(() => {
    let cancelled = false;

    async function preloadAssets() {
      try {
        console.log("[App] preloadAssets started");
        setAssets((a) => ({ ...a, xbergWasm: true }));
        console.log("[App] assets.xbergWasm = true");
        await preloadXbergWasm();
        console.log("[App] preloadXbergWasm done");

        if (await isModelCached()) {
          setAssets((a) => ({ ...a, nerModel: true }));
        } else {
          try {
            await loadNerModel();
            setAssets((a) => ({ ...a, nerModel: true }));
          } catch (e) {
            console.warn("[App] NER model download failed, using fallback:", e);
            setNerModelDegraded(true);
            setError("Neural PII backend unavailable — falling back to regex-only detection.");
            setAssets((a) => ({ ...a, nerModel: true }));
          }
        }

        setAssets((a) => ({ ...a, tessdata: true }));
        localStorage.setItem("xberg-studio-visited", "true");
      } catch (e) {
        console.error("[App] preloadAssets error:", e);
        setNerModelDegraded(true);
        setError(
          "Failed to load models — neural PII detection unavailable, falling back to regex-only detection.",
        );
        setAssets({ xbergWasm: true, nerModel: true, tessdata: true });
        localStorage.setItem("xberg-studio-visited", "true");
      }
    }

    async function init() {
      const visited = localStorage.getItem("xberg-studio-visited");
      if (visited) {
        setOnboardingComplete(true);
        setAssets({ xbergWasm: true, nerModel: true, tessdata: true });
      } else {
        await preloadAssets();
      }
      if (cancelled) return;

      const worker = new Worker(new URL("./worker/pipeline.ts", import.meta.url), {
        type: "module",
      });
      workerRef.current = worker;
      await new Promise<void>((resolve) => {
        worker.onmessage = (e) => {
          if (e.data.type === "ready") resolve();
        };
        worker.postMessage({ type: "init" });
      });
      if (cancelled) return;
      worker.onmessage = handleWorkerMessage;
      setWorkerReady(true);
    }

    function handleWorkerMessage(event: MessageEvent) {
      const { type, ...data } = event.data;
      switch (type) {
        case "progress":
          setProgress((prev) => new Map(prev).set(data.file, data as ProgressUpdate));
          break;
        case "file-complete":
          setResults((prev) => [...prev, data as ProcessedFile]);
          setProgress((prev) =>
            new Map(prev).set(data.name, { ...data, stage: "complete", percent: 100 }),
          );
          break;
        case "batch-complete":
          downloadZip(data.zip);
          // Clears the per-file progress bars (`progress` empty ⇒ `update` is undefined ⇒
          // each renders null) once the batch settles. Deliberately does NOT clear `files`
          // too: `files` also drives `FileBrowser`'s per-file row list (Track I3), which is
          // meant to persist — same as `results`, which is never cleared — so a user can
          // still see and edit a file's findings (Track I4) after this 1s delay. Clearing
          // `files` here used to make the FileBrowser row (and its `.file-edited-badge`)
          // disappear ~1s after upload, even while the "Processed this batch" edit UI below
          // stayed mounted and functional — a real bug, not a test-timing issue.
          setTimeout(() => {
            setProgress(new Map());
          }, 1000);
          break;
        case "error":
          setError(`${data.file}: ${data.message}`);
          setFileErrors((prev) => new Map(prev).set(data.file, data.message));
          break;
      }
    }

    init();
    return () => {
      cancelled = true;
      workerRef.current?.terminate();
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps -- runs once, mirrors onMount
  }, []);

  async function handleFiles(fileList: FileList | File[]): Promise<void> {
    console.log("[App] handleFiles called with", fileList.length, "files");
    const fileArray = Array.from(fileList);
    const validFiles: File[] = [];
    let skippedJunk = 0;
    const unsupported: string[] = [];

    for (const file of fileArray) {
      const name = effectiveFileName(file);
      if (isJunkFile(name)) {
        skippedJunk++;
        continue;
      }
      const validation = validateFile(file);
      console.log("[App] validateFile:", name, file.type, validation);
      if (!validation.valid) {
        unsupported.push(validation.error || "Invalid file");
        console.warn("[App] skipping unsupported file:", name, validation.error);
        continue;
      }
      validFiles.push(file);
    }

    const totalSkipped = unsupported.length + skippedJunk;
    if (fileArray.length === 1 && unsupported.length === 1) {
      // A single rejected file keeps the precise reason (existing UX contract), e.g.
      // "Unsupported file type: application/x-msdownload".
      setError(unsupported[0]);
      setSkipNotice(null);
    } else if (totalSkipped > 0) {
      // A folder's worth of rejects gets a count instead of one banner overwriting
      // itself once per file.
      const skipParts: string[] = [];
      if (unsupported.length > 0) skipParts.push(`${unsupported.length} unsupported`);
      if (skippedJunk > 0) skipParts.push(`${skippedJunk} system`);
      setSkipNotice(`Skipped ${skipParts.join(" and ")} file${totalSkipped === 1 ? "" : "s"}.`);
    } else {
      setSkipNotice(null);
    }

    if (validFiles.length === 0) return;

    setFiles((prev) => [...prev, ...validFiles]);
    setError(null);

    // Send to worker — `name` carries the folder-relative path when the file came from a
    // directory picker, so it survives into the worker's output filename (and, via
    // JSZip's own path handling, the exported zip's folder structure) rather than being
    // flattened to a basename.
    const fileInputs = await Promise.all(
      validFiles.map(async (f) => ({
        name: effectiveFileName(f),
        bytes: await f.arrayBuffer(),
        type: f.type || "application/octet-stream",
      })),
    );

    console.log("[App] posting to worker:", fileInputs.length, "files");
    workerRef.current!.postMessage({
      type: "process",
      files: fileInputs,
      config: JSON.parse(JSON.stringify(configRef.current)),
    });
  }


  function toggleFolderMode(event: React.MouseEvent): void {
    event.preventDefault();
    setFolderMode((v) => !v);
  }

  function findingsFor(result: ProcessedFile): PiiEntity[] {
    return editedFindings.get(result.name) ?? result.piiFindings;
  }

  // Track I4 "Add": a selection in `MarkdownEditor` becomes a manually-flagged finding.
  // Its `redact_template` is always a plain mask (`[CATEGORY]`), never a pseudonym token —
  // minting one would need this batch's derived key, which only ever lives inside the
  // worker (see `worker/pipeline.ts`'s `pseudonymKeyHex`), and re-deriving it here from the
  // passphrase a second time for a manual add is more machinery than this increment's
  // scope calls for. Overlapping an existing finding is silently ignored rather than
  // producing a second, conflicting decoration over the same text.
  function handleAddFinding(
    result: ProcessedFile,
    start: number,
    end: number,
    category: string,
  ): void {
    if (start >= end) return;
    setEditedFindings((prev) => {
      const current = prev.get(result.name) ?? result.piiFindings;
      if (current.some((f) => start < f.end && f.start < end)) return prev;
      const next: PiiEntity[] = [
        ...current,
        {
          category,
          text: "",
          start,
          end,
          confidence: 1,
          source: "manual",
          format_preserving: false,
          redact_template: `[${category.toUpperCase()}]`,
        },
      ].sort((a, b) => a.start - b.start);
      return new Map(prev).set(result.name, next);
    });
  }

  // Track I4 "Remove": drops a false-positive finding. Does not restore an entity link
  // `renderAnnotatedMarkdown` may have suppressed for overlapping that span in the
  // original export (Track A2's `filterExportableEntities` ran once, at export time,
  // against the original findings) — re-linking a now-unredacted span is out of scope here.
  function handleRemoveFinding(result: ProcessedFile, index: number): void {
    setEditedFindings((prev) => {
      const current = prev.get(result.name) ?? result.piiFindings;
      const next = current.filter((_, i) => i !== index);
      return new Map(prev).set(result.name, next);
    });
  }

  function handleExportEdited(result: ProcessedFile): void {
    downloadText(result.name, reExportMarkdown(result, findingsFor(result)));
  }

  // Track K2: the split view's right pane starts as the same redacted rendering the export
  // path produces, then diverges once the user edits it (see `redactedDrafts`' own comment).
  function redactedDraftFor(result: ProcessedFile): string {
    const existing = redactedDrafts.get(result.name);
    if (existing !== undefined) return existing;
    const docPath = "documents/" + result.name;
    return renderAnnotatedMarkdown(result.rawMarkdown, result.entities, findingsFor(result), docPath);
  }

  function handleRedactedChange(result: ProcessedFile, next: string): void {
    setRedactedDrafts((prev) => new Map(prev).set(result.name, next));
  }

  // `progress` is keyed by whatever name the worker was sent — the folder-relative path
  // for a directory upload, the basename otherwise — so lookups have to use the same key,
  // not `file.name`, or every folder-uploaded file's progress card silently never appears.
  const completedCount = files.filter(
    (f) => progress.get(effectiveFileName(f))?.stage === "complete",
  ).length;

  if (!onboardingComplete) {
    return (
      <Onboarding
        assets={assets}
        nerModelDegraded={nerModelDegraded}
        onComplete={() => {
          setOnboardingComplete(true);
          localStorage.setItem("xberg-studio-visited", "true");
        }}
      />
    );
  }

  return (
    <div className="flex min-h-screen flex-col">
      <header className="flex items-center justify-between border-b border-border bg-card px-6 py-3">
        <h1 className="text-xl font-semibold">Hacienda Studio</h1>
        <button
          className="config-toggle rounded-md bg-muted px-4 py-2 text-sm text-foreground transition-colors hover:bg-primary hover:text-primary-foreground"
          aria-expanded={showConfig}
          onClick={() => setShowConfig((v) => !v)}
        >
          ⚙ Config
        </button>
      </header>

      {error && (
        <div
          className="error-banner flex items-center justify-between bg-destructive/15 px-6 py-3 text-destructive"
          role="alert"
        >
          <span>❌ {error}</span>
          <button aria-label="Dismiss" className="text-lg leading-none" onClick={() => setError(null)}>
            ✕
          </button>
        </div>
      )}

      {skipNotice && (
        <div
          className="skip-notice flex items-center justify-between border-b border-border bg-card px-6 py-3 text-sm text-muted-foreground"
          role="status"
        >
          <span>ℹ️ {skipNotice}</span>
          <button
            aria-label="Dismiss"
            className="text-lg leading-none"
            onClick={() => setSkipNotice(null)}
          >
            ✕
          </button>
        </div>
      )}

      <main className="mx-auto w-full max-w-[800px] flex-1 px-6 py-12">
        <section aria-label="Upload documents">
          <FileUpload
            className="drop-zone"
            id="file-input"
            disabled={!workerReady}
            multiple
            filterAccept={false}
            showFileList={false}
            showBorderBeam={workerReady}
            webkitdirectory={folderMode}
            accept={folderMode ? undefined : UPLOAD_ACCEPT}
            inputAriaLabel={folderMode ? "Choose a folder" : "Choose files"}
            title={
              workerReady
                ? folderMode
                  ? "Drop a folder here or click to browse"
                  : "Drop files here or click to browse"
                : "Starting the local engine…"
            }
            description="PDF, Office, Email, Images, Audio/Video, Subtitles, Code — up to 50MB each"
            onFilesAccepted={(accepted) => handleFiles(accepted)}
          />
          <button
            type="button"
            className="mode-toggle mx-auto mt-4 block w-fit bg-transparent text-xs text-muted-foreground underline hover:text-primary disabled:cursor-not-allowed disabled:opacity-60"
            disabled={!workerReady}
            onClick={toggleFolderMode}
          >
            {folderMode ? "or choose individual files" : "or choose a folder"}
          </button>
        </section>

        {files.length > 0 && (
          <>
            {files.length > 1 && (
              <p className="batch-summary -mb-2 mt-6 text-center text-sm text-muted-foreground" aria-live="polite">
                {completedCount} / {files.length} processed
              </p>
            )}
            <section className="mt-6 flex flex-col gap-3" aria-live="polite">
              {files.map((file) => {
                const key = effectiveFileName(file);
                const update = progress.get(key);
                if (!update) return null;
                return <ProgressBar key={key} file={{ name: key }} update={update} />;
              })}
            </section>
          </>
        )}

        {files.length > 0 && (
          <FileBrowser
            rows={buildFileRows(
              files,
              progress,
              results,
              fileErrors,
              new Set(editedFindings.keys()),
              new Map(results.map((r) => [r.name, findingsFor(r)] as const)),
            )}
            openName={openResultName}
            onOpen={(name) => setOpenResultName((current) => (current === name ? null : name))}
          />
        )}

        {results.length > 0 && (
          <section className="mt-8 flex flex-col gap-4 border-t border-border pt-6">
            <h2 className="text-sm font-semibold text-muted-foreground">
              Processed this batch ({results.length})
            </h2>
            {results.map((result) => {
              const findings = findingsFor(result);
              const edited = editedFindings.has(result.name);
              const viewerKind = getViewerKind(result.frontmatter.source);
              const previewUrl = previewUrls.get(result.name);
              // `openResultName` is set from `FileBrowser`'s row name, which is the original
              // input file name (`frontmatter.source`, e.g. "note.docx") — not `result.name`,
              // which is the output markdown's own name (e.g. "note.md"). Comparing against
              // `result.name` here would never match and the split view would never open.
              const isOpen = openResultName === result.frontmatter.source;

              if (isOpen && viewerKind && previewUrl) {
                return (
                  <div key={result.name} className="flex flex-col gap-2">
                    <div className="flex items-center justify-between text-sm">
                      <span className="font-medium">{result.name}</span>
                      <button
                        type="button"
                        className="close-split-view rounded-md bg-muted px-2 py-1 text-xs text-foreground hover:bg-primary hover:text-primary-foreground"
                        onClick={() => setOpenResultName(null)}
                      >
                        ✕ Close
                      </button>
                    </div>
                    <div className="split-view grid grid-cols-1 gap-4 md:grid-cols-2">
                      <div className="min-w-0">
                        {viewerKind === "docx" && (
                          <DocxViewerPreview
                            src={previewUrl}
                            fileName={result.name}
                            isDark={viewerDark}
                            onIsDarkChange={setViewerDark}
                            showUpload={false}
                          />
                        )}
                        {viewerKind === "xlsx" && (
                          <XlsxViewerPreview
                            src={previewUrl}
                            fileName={result.name}
                            isDark={viewerDark}
                            onIsDarkChange={setViewerDark}
                            showUpload={false}
                          />
                        )}
                        {viewerKind === "pptx" && (
                          <PptxViewerPreview src={previewUrl} fileName={result.name} showUpload={false} />
                        )}
                        {viewerKind === "pdf" && (
                          <PDFViewer src={previewUrl} fileName={result.name} showUpload={false} />
                        )}
                      </div>
                      <div className="min-w-0">
                        <RedactedEditor
                          value={redactedDraftFor(result)}
                          onChange={(next) => handleRedactedChange(result, next)}
                        />
                      </div>
                    </div>
                  </div>
                );
              }

              return (
                <div key={result.name} className="flex flex-col gap-2">
                  <div className="flex items-center justify-between text-sm">
                    <span className="font-medium">{result.name}</span>
                    <div className="flex items-center gap-3 text-muted-foreground">
                      <span>
                        {result.entities.length}{" "}
                        {result.entities.length === 1 ? "entity" : "entities"}
                      </span>
                      {edited && (
                        <button
                          type="button"
                          className="export-edited rounded-md bg-muted px-2 py-1 text-xs text-foreground hover:bg-primary hover:text-primary-foreground"
                          onClick={() => handleExportEdited(result)}
                        >
                          Export edited file
                        </button>
                      )}
                    </div>
                  </div>
                  {viewerKind === "docx" && previewUrl && (
                    <DocxViewerPreview
                      src={previewUrl}
                      fileName={result.name}
                      isDark={viewerDark}
                      onIsDarkChange={setViewerDark}
                      showUpload={false}
                    />
                  )}
                  {viewerKind === "xlsx" && previewUrl && (
                    <XlsxViewerPreview
                      src={previewUrl}
                      fileName={result.name}
                      isDark={viewerDark}
                      onIsDarkChange={setViewerDark}
                      showUpload={false}
                    />
                  )}
                  {viewerKind === "pptx" && previewUrl && (
                    <PptxViewerPreview src={previewUrl} fileName={result.name} showUpload={false} />
                  )}
                  {viewerKind === "pdf" && previewUrl && (
                    <PDFViewer src={previewUrl} fileName={result.name} showUpload={false} />
                  )}
                  <MarkdownEditor
                    value={result.rawMarkdown}
                    findings={findings}
                    onAddFinding={(start, end, category) =>
                      handleAddFinding(result, start, end, category)
                    }
                  />
                  {(findings.length > 0 || edited) && (
                    <PiiPanel
                      findings={findings}
                      onRemove={(i) => handleRemoveFinding(result, i)}
                    />
                  )}
                </div>
              );
            })}
          </section>
        )}
      </main>

      {showConfig && (
        <ConfigPanel config={config} onChange={setConfig} onDone={() => setShowConfig(false)} />
      )}
    </div>
  );
}
