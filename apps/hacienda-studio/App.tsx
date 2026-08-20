import { useEffect, useRef, useState } from "react";
import { ConfigPanel } from "./components/ConfigPanel";
import { Landing } from "./pages/Landing";
import { Assets } from "./pages/Assets";
import { Studio } from "./pages/Studio";
import { Documents } from "./pages/Documents";
import { DocumentDetail } from "./pages/DocumentDetail";
import { loadNerModel, isModelCached, preloadXbergWasm, validateFile } from "./lib/asset-loader";
import { WorkerPool, createWorkerPool } from "./lib/worker-pool";
import { effectiveFileName, isJunkFile } from "./lib/file-filter";
import {
  saveDocument,
  saveEditedFindings,
  deleteDocument,
  listDocuments,
  listEditedFindings,
} from "./lib/persistence";
import { pruneDrafts } from "./lib/redaction-store";
import { folderOf } from "./lib/library";
import type { LibraryDocument } from "./lib/library";
import { DEFAULT_CONFIG } from "./lib/types";
import type { AppConfig, OnboardingState, ProcessedFile, ProgressUpdate } from "./lib/types";
import type { PiiEntity } from "./lib/pii-engine";
// Type-only: erased at compile time, so this does not pull `@remotion/whisper-web` into the
// bundle just for the type. The runtime class is loaded with a dynamic `import()` the first
// time a "transcribe-request" actually arrives (see `handleTranscribeRequest` below) — most
// sessions never enable transcription, and whisper-web's wasm/JS should not load for them.
import type { WhisperBridge } from "./lib/transcription/whisper-bridge";
import type { TranscriptionConfig, TranscriptionResult } from "./lib/transcription/types";

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
 * Wraps an already-spliced redacted body (`DocumentDetail`'s free-text draft, or a
 * mode-recomputed rendering) with `result.markdown`'s original frontmatter and entity
 * glossary. Uses the same `lastIndexOf`-based glossary extraction `lib/export-resolve.ts`'s
 * `reExportMarkdown` documents: a first-match regex would misfire if the source document's
 * own body happens to contain a literal "## Entities" heading before the pipeline's
 * appended one.
 */
function wrapRedactedBody(result: ProcessedFile, body: string): string {
  const frontmatterMatch = result.markdown.match(/^---\n[\s\S]*?\n---/);
  const frontmatter = frontmatterMatch ? frontmatterMatch[0] : "";
  const glossaryMarker = "\n## Entities\n\n";
  const glossaryStart = result.markdown.lastIndexOf(glossaryMarker);
  const glossary = glossaryStart === -1 ? "" : result.markdown.slice(glossaryStart);
  return frontmatter + "\n" + body + glossary;
}

type Route = "landing" | "assets" | "studio";
type StudioView = "upload" | "documents" | "document";

export function App() {
  const [route, setRoute] = useState<Route>("landing");
  const [studioView, setStudioView] = useState<StudioView>("upload");
  const [selectedDocumentName, setSelectedDocumentName] = useState<string | null>(null);
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

  const workerPoolRef = useRef<WorkerPool | null>(null);
  const configRef = useRef(config);
  configRef.current = config;
  // Track D3's follow-up (see `worker/transcribe-bridge.ts`'s header): the one `WhisperBridge`
  // instance for this tab, created lazily on the first "transcribe-request" the worker sends
  // and reused for every request after that — instance reuse is what makes `load()`'s
  // idempotency (skip re-download/re-init once a model is loaded) actually apply across a
  // whole batch, and across batches, instead of just within a single call.
  const whisperBridgeRef = useRef<WhisperBridge | null>(null);

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
        setAssets({ xbergWasm: true, nerModel: true, tessdata: true });
      } else {
        await preloadAssets();
      }
      if (cancelled) return;

      // Initialize worker pool with transcription handler. Every pool worker can send a
      // "transcribe-request" (whisper can't run inside a worker — see
      // `worker/transcribe-bridge.ts`'s header); `respond` is bound by `WorkerPool` to
      // the *specific* worker instance that asked, so there's no separate "just for
      // transcription" worker needed — that used to mean an extra full model load.
      const pool = await createWorkerPool(
        { poolSize: 3 },
        async (data, respond) => {
          await handleTranscribeRequest(
            data as {
              requestId: string;
              file: string;
              audioBytes: Uint8Array<ArrayBuffer>;
              mimeType: string;
              config: TranscriptionConfig;
            },
            respond,
          );
        }
      );
      workerPoolRef.current = pool;

      // Set up pool callbacks
      pool.setProgressHandler((update) => {
        setProgress((prev) => new Map(prev).set(update.file, update));
      });
      pool.setFileCompleteHandler((result) => {
        setResults((prev) => [...prev, result]);
        setProgress((prev) =>
          new Map(prev).set(result.name, { ...result, stage: "complete", percent: 100 })
        );
        void saveDocument(result);
      });
      pool.setErrorHandler((file, error) => {
        setError(`${file}: ${error}`);
        setFileErrors((prev) => new Map(prev).set(file, error));
      });
      pool.setWarningHandler((message) => {
        console.warn("[WorkerPool] Warning:", message);
        // Could show a toast notification here
      });
      pool.setBatchCompleteHandler(() => {
        // Batch complete - transition to documents view after a delay
        setTimeout(() => {
          setProgress(new Map());
          setStudioView("documents");
        }, 1000);
      });

      setWorkerReady(true);
    }

    // Track D3's fix, completed: the worker cannot run `@remotion/whisper-web` itself (see
    // `worker/transcribe-bridge.ts`'s header for exactly why) so it asks this main thread to
    // do it instead, and this is that request's handler — the other half of the RPC whose
    // worker side is `TranscriptionRequestBridge.request()`. Always replies with a
    // "transcribe-response" carrying either `result` or `error`, even when something here
    // throws before `WhisperBridge` itself runs (e.g. the dynamic `import()` failing) —
    // an unanswered request is exactly the failure mode `transcriptionBridge`'s own timeout
    // exists to catch, but answering promptly here means that file's real error reaches the
    // UI in milliseconds, not after a 15-minute wait.
    async function handleTranscribeRequest(
      data: {
        requestId: string;
        file: string;
        audioBytes: Uint8Array<ArrayBuffer>;
        mimeType: string;
        config: TranscriptionConfig;
      },
      respond: (response: { result?: unknown; error?: string }) => void,
    ): Promise<void> {
      try {
        if (!whisperBridgeRef.current) {
          const { WhisperBridge } = await import("./lib/transcription/whisper-bridge");
          whisperBridgeRef.current = new WhisperBridge();
        }
        const result: TranscriptionResult = await whisperBridgeRef.current.transcribeAudio(
          data.audioBytes,
          data.mimeType,
          data.config,
          (phase, fraction) => {
            // Resample and transcribe share this file's progress card, split into two
            // halves of the same 0-100 bar (10 is where the worker's own "extract" stage
            // left off for an audio/video file; 60 is where its "ner" stage picks back up
            // once this resolves — see `processFile`'s transcription branch) — an
            // approximation (resampling is not really a fixed 30% of the work), but it
            // turns an opaque wait into a moving bar with a real, monotonically increasing
            // percentage instead of a fabricated one.
            const percent =
              phase === "resample" ? 10 + fraction * 30 : 40 + fraction * 50;
            setProgress((prev) =>
              new Map(prev).set(data.file, {
                file: data.file,
                stage: "transcribe",
                percent,
                message:
                  phase === "resample"
                    ? `Resampling audio (${Math.round(fraction * 100)}%)`
                    : `Transcribing (${Math.round(fraction * 100)}%)`,
              }),
            );
          },
        );
        respond({ result });
      } catch (e) {
        respond({ error: e instanceof Error ? e.message : "Unknown error" });
      }
    }

    init();
    return () => {
      cancelled = true;
      workerPoolRef.current?.terminate();
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps -- runs once, mirrors onMount
  }, []);

  // Redesign: hydrate the document library from IndexedDB on mount, so documents
  // processed in a previous tab session still show up in the Documents page. Read-only
  // merge into `results`/`editedFindings` — this session's own worker output (via
  // `saveDocument`/`saveEditedFindings` above) is what keeps storage in sync going
  // forward, not a re-read here.
  useEffect(() => {
    let cancelled = false;
    async function hydrate() {
      const [persistedDocs, persistedEdits] = await Promise.all([
        listDocuments(),
        listEditedFindings(),
      ]);
      if (cancelled || persistedDocs.length === 0) return;
      setResults((prev) => {
        const existing = new Set(prev.map((r) => r.name));
        const toAdd = persistedDocs
          .map((d) => d.result)
          .filter((r) => !existing.has(r.name));
        return toAdd.length > 0 ? [...prev, ...toAdd] : prev;
      });
      if (persistedEdits.size > 0) {
        setEditedFindings((prev) => new Map([...persistedEdits, ...prev]));
      }
      setStudioView((v) => (v === "upload" ? "documents" : v));
    }
    hydrate();
    return () => {
      cancelled = true;
    };
  }, []);

  // Redacted-markdown drafts (`DocumentDetail`'s free-text autosave) can end up holding
  // PII a user left in place or pasted unredacted — bound their local retention by
  // default rather than only via an explicit "clear" action. Runs once per app load, not
  // once per draft, so a draft never outlives 30 days of the app simply never reopening.
  useEffect(() => {
    void pruneDrafts(30 * 24 * 60 * 60 * 1000);
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

    console.log("[App] posting to worker pool:", fileInputs.length, "files");
    const pool = workerPoolRef.current;
    if (!pool) {
      setError("Worker pool not initialized");
      return;
    }
    
    try {
      const results = await pool.processFiles(fileInputs, JSON.parse(JSON.stringify(configRef.current)));
      // Results are handled via callbacks, but we can also use the returned array
      console.log("[App] Worker pool completed:", results.length, "files");
    } catch (error) {
      console.error("[App] Worker pool error:", error);
      setError(error instanceof Error ? error.message : "Worker pool processing failed");
    }
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
      void saveEditedFindings(result.name, next);
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
      void saveEditedFindings(result.name, next);
      return new Map(prev).set(result.name, next);
    });
  }

  // Redesign: removes a document from both the in-memory library and the persisted
  // one — used by the Documents page's bulk delete. `files`/`progress` are left alone:
  // they're this-session upload-queue state, not the library, and a document deleted
  // here may not even correspond to an entry in them (e.g. one hydrated from a prior
  // session).
  function handleDeleteDocuments(names: string[]): void {
    const nameSet = new Set(names);
    setResults((prev) => prev.filter((r) => !nameSet.has(r.name)));
    setEditedFindings((prev) => {
      const next = new Map(prev);
      for (const name of names) next.delete(name);
      return next;
    });
    for (const name of names) void deleteDocument(name);
  }

  function goToStudio(): void {
    setRoute("studio");
    setStudioView(results.length > 0 ? "documents" : "upload");
  }

  function openDocument(name: string): void {
    setSelectedDocumentName(name);
    setStudioView("document");
  }

  const libraryDocuments: LibraryDocument[] = results.map((result) => ({
    result,
    findings: findingsFor(result),
    folder: folderOf(result.name),
    baseName: result.name,
  }));

  const selectedResult = selectedDocumentName
    ? results.find((r) => r.name === selectedDocumentName)
    : undefined;
  // The in-memory `File` this result came from, when this session's the one that
  // processed it — undefined once hydrated from a prior session's persisted library,
  // where only the processed output survives. `DocumentDetail` uses its presence/absence
  // to gate the native-viewer split view and the redacted-draft autosave, both of which
  // need real file bytes.
  const selectedOriginalFile = selectedResult
    ? files.find((f) => effectiveFileName(f) === selectedResult.frontmatter.source)
    : undefined;

  const navItem = (label: string, active: boolean, onClick: () => void) => (
    <button
      key={label}
      type="button"
      className={`rounded-md px-3 py-1.5 text-sm transition-colors ${
        active
          ? "bg-muted font-medium text-foreground"
          : "text-muted-foreground hover:bg-muted/60 hover:text-foreground"
      }`}
      onClick={onClick}
    >
      {label}
    </button>
  );

  return (
    <div className="flex min-h-screen flex-col bg-background text-foreground">
      <header className="flex items-center justify-between border-b border-border bg-card px-6 py-3">
        <div className="flex items-center gap-4">
          <h1 className="text-xl font-semibold">Hacienda Studio</h1>
          <nav aria-label="App sections" className="flex items-center gap-1">
            {navItem("Studio", route === "studio", goToStudio)}
            {navItem("Assets", route === "assets", () => setRoute("assets"))}
            {navItem("Chat", route === "chat", () => setRoute("chat"))}
          </nav>
        </div>
        <button
          className="config-toggle rounded-md bg-muted px-4 py-2 text-sm text-foreground transition-colors hover:bg-primary hover:text-primary-foreground"
          aria-expanded={showConfig}
          onClick={() => setShowConfig((v) => !v)}
        >
          ⚙ Settings
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

      {route === "landing" && (
        <Landing onPrepare={() => setRoute("assets")} onSkip={goToStudio} />
      )}

      {route === "assets" && (
        <Assets assets={assets} nerModelDegraded={nerModelDegraded} onContinue={goToStudio} />
      )}

      {route === "chat" && <ChatPage />}

      {route === "studio" && studioView === "upload" && (
        <Studio
          workerReady={workerReady}
          folderMode={folderMode}
          onToggleFolderMode={toggleFolderMode}
          onFilesAccepted={(accepted) => handleFiles(accepted)}
          files={files}
          progress={progress}
          results={results}
          fileErrors={fileErrors}
          onOpenDocument={openDocument}
        />
      )}

      {route === "studio" && studioView === "documents" && (
        <Documents
          documents={libraryDocuments}
          onOpenDocument={openDocument}
          onDeleteDocuments={handleDeleteDocuments}
          onAddFiles={() => setStudioView("upload")}
        />
      )}

      {route === "studio" && studioView === "document" && selectedResult && (
        <DocumentDetail
          result={selectedResult}
          findings={findingsFor(selectedResult)}
          originalFile={selectedOriginalFile}
          onBack={() => setStudioView("documents")}
          onAddFinding={(start, end, category) =>
            handleAddFinding(selectedResult, start, end, category)
          }
          onRemoveFinding={(i) => handleRemoveFinding(selectedResult, i)}
          onExportBody={(body) =>
            downloadText(selectedResult.name, wrapRedactedBody(selectedResult, body))
          }
        />
      )}

      {showConfig && (
        <ConfigPanel config={config} onChange={setConfig} onDone={() => setShowConfig(false)} />
      )}
    </div>
  );
}
