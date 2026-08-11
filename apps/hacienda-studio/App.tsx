import { useEffect, useRef, useState } from "react";
import { ConfigPanel } from "./components/ConfigPanel";
import { buildFileRows } from "./components/FileBrowser";
import { AssetLoadingScreen } from "./components/screens/AssetLoadingScreen";
import { UploadScreen } from "./components/screens/UploadScreen";
import { QueueScreen } from "./components/screens/QueueScreen";
import { FileBrowserScreen } from "./components/screens/FileBrowserScreen";
import { DetailScreen } from "./components/screens/DetailScreen";
import {
  loadNerModel,
  isModelCached,
  preloadXbergWasm,
  validateFile,
  type DownloadProgress,
} from "./lib/asset-loader";
import { effectiveFileName, isJunkFile } from "./lib/file-filter";
import { renderAnnotatedMarkdown } from "./lib/annotate";
import { reExportMarkdown, resolveExportContent } from "./lib/export-resolve";
import { getViewerKind } from "./lib/viewer-kind";
import { computeContentHash } from "./lib/content-hash";
import { loadDraft, saveDraft } from "./lib/redaction-store";
import { DEFAULT_CONFIG } from "./lib/types";
import type { Screen } from "./lib/screens";
import type { AppConfig, OnboardingState, ProcessedFile, ProgressUpdate } from "./lib/types";
import type { PiiEntity } from "./lib/pii-engine";

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

export function App() {
  // Replaces the prior `onboardingComplete: boolean` + `openResultName: string | null`
  // pair — see `lib/screens.ts`'s header for why folding both into one discriminated
  // union removes a whole class of "which of two names is authoritative" bug.
  const [screen, setScreen] = useState<Screen>({ kind: "asset-loading" });
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
  // Byte-level NER model download progress, threaded into `AssetLoadingScreen` for the
  // "clear loading progress" requirement — null whenever no download is in flight
  // (cached model, not-yet-started, or finished/failed).
  const [nerDownloadProgress, setNerDownloadProgress] = useState<DownloadProgress | null>(null);
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
  // Track K2: the redacted pane's free-text buffer per result, keyed by `ProcessedFile.name`.
  // Seeded lazily from `renderAnnotatedMarkdown` the first time a result's split view opens
  // (see `redactedDraftFor` below); once present, edits here are independent of
  // `piiFindings`/`entities` offsets, same scope cut as `MarkdownEditor`'s own edit model.
  const [redactedDrafts, setRedactedDrafts] = useState<Map<string, string>>(new Map());
  // Track K/Phase 4: SHA-256 hex of each input file's bytes, keyed by `effectiveFileName`
  // (the same key `progress`/`fileErrors` use) — computed once in `handleFiles` from bytes
  // already being read for the worker upload, and used as the IndexedDB autosave key so a
  // draft survives a reload without being tied to a filename.
  const [contentHashes, setContentHashes] = useState<Map<string, string>>(new Map());
  // Autosave status for whichever result the detail screen currently has open — null
  // outside the detail screen or before any edit has triggered a save.
  const [saveStatus, setSaveStatus] = useState<"saving" | "saved" | null>(null);

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
        if (!file) {
          console.warn(`[App] No matching file for result ${result.name} (source: ${result.frontmatter.source}). files:`, files.map(f => effectiveFileName(f)));
          continue;
        }
        console.log(`[App] Creating preview URL for ${result.name} from file ${file.name}`);
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
            await loadNerModel((p) => setNerDownloadProgress(p));
            setAssets((a) => ({ ...a, nerModel: true }));
          } catch (e) {
            console.warn("[App] NER model download failed, using fallback:", e);
            setNerModelDegraded(true);
            setError("Neural PII backend unavailable — falling back to regex-only detection.");
            setAssets((a) => ({ ...a, nerModel: true }));
          } finally {
            setNerDownloadProgress(null);
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
        setScreen({ kind: "upload" });
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
          // queue → browser: the worker has settled every file in this batch. Zip
          // building is now a deliberate, on-demand action (the file-browser screen's
          // "Download redacted zip" button, via handleDownloadZip's "build-zip" message)
          // rather than something that runs unconditionally here.
          setScreen({ kind: "browser" });
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
          console.error("[App] Worker error received:", data);
          console.error("[App] Error file:", data.file);
          console.error("[App] Error message:", data.message);
          setError(`${data.file}: ${data.message}`);
          setFileErrors((prev) => new Map(prev).set(data.file, data.message));
          break;
        case "zip-ready":
          downloadZip(data.zip);
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
    // flattened to a basename. `hash` rides along on the same `arrayBuffer()` read (Track
    // K/Phase 4's autosave key) rather than re-reading each file a second time, and is
    // stripped back out before posting — `FileInput` (`lib/types.ts`) has no `hash` field.
    const fileInputs = await Promise.all(
      validFiles.map(async (f) => {
        const bytes = await f.arrayBuffer();
        const hash = await computeContentHash(bytes);
        return { name: effectiveFileName(f), bytes, type: f.type || "application/octet-stream", hash };
      }),
    );
    setContentHashes((prev) => {
      const next = new Map(prev);
      for (const fi of fileInputs) next.set(fi.name, fi.hash);
      return next;
    });

    console.log("[App] posting to worker:", fileInputs.length, "files");
    workerRef.current!.postMessage({
      type: "process",
      files: fileInputs.map(({ name, bytes, type }) => ({ name, bytes, type })),
      config: JSON.parse(JSON.stringify(configRef.current)),
    });
    setScreen({ kind: "queue" });
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

  // Track K/Phase 2: "Download redacted zip" — a worker round-trip rather than a
  // main-thread zip rebuild, since only the worker retains the live `BatchEntityRegistry`
  // instance (`lastBatch` in `worker/pipeline.ts`) that `assembleZip` needs. `overrides`
  // only carries entries that actually diverge from `result.markdown`, so files nobody
  // touched still export exactly what the pipeline produced.
  function handleDownloadZip(): void {
    const overrides: Record<string, string> = {};
    for (const result of results) {
      const content = resolveExportContent(result, redactedDrafts, editedFindings);
      if (content !== result.markdown) overrides[result.name] = content;
    }
    workerRef.current!.postMessage({ type: "build-zip", overrides });
  }

  function handleDownloadFile(result: ProcessedFile): void {
    const content = resolveExportContent(result, redactedDrafts, editedFindings);
    const blob = new Blob([content], { type: "text/markdown" });
    const url = URL.createObjectURL(blob);
    const a = document.createElement("a");
    a.href = url;
    a.download = result.name;
    document.body.appendChild(a);
    a.click();
    document.body.removeChild(a);
    URL.revokeObjectURL(url);
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

  const openDetailResult =
    screen.kind === "detail" ? results.find((r) => r.frontmatter.source === screen.inputName) : undefined;

  // Track K/Phase 4: restore-on-open. Fires once per detail-screen open, looks up a saved
  // draft by content hash, and adopts it only if `redactedDrafts` still has no entry for
  // this result by the time the lookup resolves — `redactedDraftFor`'s synchronous
  // `renderAnnotatedMarkdown` fallback already renders immediately (no blank flash), and an
  // in-session edit (including a first keystroke that lands before this resolves) always
  // wins over a stale on-disk draft rather than being silently clobbered by it.
  useEffect(() => {
    if (!openDetailResult) return;
    const result = openDetailResult;
    const hash = contentHashes.get(result.frontmatter.source);
    if (!hash) return;
    let cancelled = false;
    loadDraft(hash).then((saved) => {
      if (cancelled || saved === undefined) return;
      setRedactedDrafts((prev) => (prev.has(result.name) ? prev : new Map(prev).set(result.name, saved)));
    });
    return () => {
      cancelled = true;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps -- keyed on which file is open, not on redactedDrafts/results identity
  }, [openDetailResult?.name]);

  // Track K/Phase 4: debounced (~1s) autosave of the open detail screen's own draft. Only
  // ever watches `openDetailResult`'s entry in `redactedDrafts` — while the detail screen
  // is open, that is the only entry `handleRedactedChange` can touch.
  useEffect(() => {
    if (!openDetailResult) {
      setSaveStatus(null);
      return;
    }
    const result = openDetailResult;
    const hash = contentHashes.get(result.frontmatter.source);
    const draft = redactedDrafts.get(result.name);
    if (!hash || draft === undefined) return;
    setSaveStatus("saving");
    const timer = setTimeout(() => {
      saveDraft(hash, draft).then(() => setSaveStatus("saved"));
    }, 1000);
    return () => clearTimeout(timer);
  }, [openDetailResult, contentHashes, redactedDrafts]);

  // `progress` is keyed by whatever name the worker was sent — the folder-relative path
  // for a directory upload, the basename otherwise — so lookups have to use the same key,
  // not `file.name`, or every folder-uploaded file's progress card silently never appears.
  const completedCount = files.filter(
    (f) => progress.get(effectiveFileName(f))?.stage === "complete",
  ).length;

  if (screen.kind === "asset-loading") {
    return (
      <AssetLoadingScreen
        assets={assets}
        nerModelDegraded={nerModelDegraded}
        nerDownloadProgress={nerDownloadProgress}
        onComplete={() => {
          setScreen({ kind: "upload" });
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

      <main className="flex flex-1 flex-col">
        {screen.kind === "upload" && (
          <UploadScreen
            workerReady={workerReady}
            folderMode={folderMode}
            onToggleFolderMode={toggleFolderMode}
            onFilesAccepted={(accepted) => handleFiles(accepted)}
          />
        )}

        {screen.kind === "queue" && (
          <QueueScreen files={files} progress={progress} completedCount={completedCount} />
        )}

        {screen.kind === "browser" && (
          <FileBrowserScreen
            rows={buildFileRows(
              files,
              progress,
              results,
              fileErrors,
              new Set(editedFindings.keys()),
              new Map(results.map((r) => [r.name, findingsFor(r)] as const)),
            )}
            onOpen={(name) => setScreen({ kind: "detail", inputName: name })}
            onAddMore={() => setScreen({ kind: "upload" })}
            onDownloadZip={handleDownloadZip}
            onDownloadFile={handleDownloadFile}
            previewUrls={previewUrls}
          />
        )}

        {screen.kind === "detail" &&
          (() => {
            // `openDetailResult` already resolves `screen.inputName` (the original input
            // file name, e.g. "note.docx") against `result.frontmatter.source` — not
            // `result.name`, the output markdown's own name (e.g. "note.md"), which would
            // never match.
            const result = openDetailResult;
            if (!result) return null;
            const findings = findingsFor(result);
            const edited = editedFindings.has(result.name);
            return (
              <DetailScreen
                result={result}
                viewerKind={getViewerKind(result.frontmatter.source)}
                previewUrl={previewUrls.get(result.name)}
                viewerDark={viewerDark}
                onViewerDarkChange={setViewerDark}
                redactedValue={redactedDraftFor(result)}
                onRedactedChange={(next) => handleRedactedChange(result, next)}
                saveStatus={saveStatus}
                findings={findings}
                edited={edited}
                onAddFinding={(start, end, category) =>
                  handleAddFinding(result, start, end, category)
                }
                onRemoveFinding={(i) => handleRemoveFinding(result, i)}
                onExportEdited={() => handleExportEdited(result)}
                onDownloadZip={handleDownloadZip}
                onBack={() => setScreen({ kind: "browser" })}
              />
            );
          })()}
      </main>

      {showConfig && (
        <ConfigPanel config={config} onChange={setConfig} onDone={() => setShowConfig(false)} />
      )}
    </div>
  );
}
