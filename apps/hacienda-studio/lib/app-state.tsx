import {
  createContext,
  useContext,
  useEffect,
  useRef,
  useState,
  type ReactNode,
} from "react";
import { toast } from "sonner";
import {
  loadNerModel,
  isModelCached,
  preloadXbergWasm,
  loadTessdata,
  validateFile,
  checkPdfPageSafety,
} from "./asset-loader";
import { WorkerPool, createWorkerPool } from "./worker-pool";
import { detectDeviceTier, poolSizeForTier } from "./device-tier";
import { effectiveFileName, isJunkFile } from "./file-filter";
import {
  saveDocument,
  saveEditedFindings,
  deleteDocument,
  listDocuments,
  listEditedFindings,
} from "./persistence";
import { pruneDrafts } from "./redaction-store";
import { recordKeyUsage } from "./pseudonym-keys";
import { folderOf } from "./library";
import type { LibraryDocument } from "./library";
import { DEFAULT_CONFIG } from "./types";
import type { AppConfig, NerCategory, OnboardingState, ProcessedFile, ProgressUpdate } from "./types";
import type { PiiEntity } from "./pii-engine";
import type { WhisperBridge } from "./transcription/whisper-bridge";
import type { TranscriptionConfig, TranscriptionResult } from "./transcription/types";
import { createSession } from "./session";
import type { Session, TreatmentMode } from "./session";
import type { ConversionMode } from "./conversion";
import { categoryToWire } from "./pii-categories";

function downloadZip(blob: Blob): void {
  const url = URL.createObjectURL(blob);
  const a = document.createElement("a");
  a.href = url;
  a.download = `xberg-output-${Date.now()}.zip`;
  a.click();
  // Deferred, not immediate: `a` is never attached to the document, so the browser
  // fetches the blob URL asynchronously after `.click()` returns — revoking
  // synchronously here can race that fetch and produce an empty download.
  setTimeout(() => URL.revokeObjectURL(url), 0);
}

function downloadText(fileName: string, content: string): void {
  const blob = new Blob([content], { type: "text/markdown" });
  const url = URL.createObjectURL(blob);
  const a = document.createElement("a");
  a.href = url;
  a.download = fileName;
  a.click();
  setTimeout(() => URL.revokeObjectURL(url), 0);
}

const NER_CATEGORIES_SET: ReadonlySet<string> = new Set([
  "person",
  "organization",
  "location",
  "date",
  "time",
  "money",
  "percent",
  "email",
  "phone",
  "url",
]);

function isNerCategory(value: string): value is NerCategory {
  return NER_CATEGORIES_SET.has(value);
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

/**
 * Track E3/router migration: this hook is the entire body of the former `App()` component
 * minus its JSX — every `useState`/`useRef`/`useEffect`/handler that used to live in
 * `App.tsx`, unchanged in substance. `route`/`studioView` are gone; the router (see
 * `router.tsx`, `routes/`) owns navigation now, this hook owns everything else. Route
 * components read this via `useAppState()` below instead of receiving it as props.
 */
function useAppStateProvider() {
  const [assets, setAssets] = useState<OnboardingState["assets"]>({
    xbergWasm: false,
    nerModel: false,
    tessdata: false,
  });
  const [nerModelProgress, setNerModelProgress] = useState<{ receivedBytes: number; totalBytes: number | null } | null>(null);
  const [files, setFiles] = useState<File[]>([]);
  // Redesign: files land here first — reviewable, removable — and only move into `files`
  // (which drives the worker pool) once the user confirms via `handleProcessQueue`.
  const [pendingFiles, setPendingFiles] = useState<File[]>([]);
  const [progress, setProgress] = useState<Map<string, ProgressUpdate>>(new Map());
  const [results, setResults] = useState<ProcessedFile[]>([]);
  const [config, setConfig] = useState<AppConfig>({ ...DEFAULT_CONFIG });
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

  // Task 3: current session for Vue d'ensemble panels. Defaults align with screenshot:
  // 14 categories, pseudonymize, markdown. Persisting to config via setters below.
  const [session, setSession] = useState<Session>(() => createSession("Session du 26/08"));

  const workerPoolRef = useRef<WorkerPool | null>(null);
  const configRef = useRef(config);
  configRef.current = config;
  const progressRef = useRef(progress);
  progressRef.current = progress;
  // Track D3's follow-up (see `worker/transcribe-bridge.ts`'s header): the one `WhisperBridge`
  // instance for this tab, created lazily on the first "transcribe-request" the worker sends
  // and reused for every request after that — instance reuse is what makes `load()`'s
  // idempotency (skip re-download/re-init once a model is loaded) actually apply across a
  // whole batch, and across batches, instead of just within a single call.
  const whisperBridgeRef = useRef<WhisperBridge | null>(null);
  // Populated by `checkPdfPageSafety` in `handleFilesAccepted` for PDFs whose page count
  // makes OCR a memory risk; read back in `handleProcessQueue` when building each file's
  // `FileInput`. Keyed by the `File` object itself (stable across `pendingFiles`/`files`
  // state, which hold the same references) rather than by name, since folder uploads can
  // have same-named siblings in different subdirectories.
  const disableOcrForFileRef = useRef<WeakMap<File, boolean>>(new WeakMap());
  // Same pattern as `disableOcrForFileRef`, for the pdfium page count `checkPdfPageSafety`
  // already computed — `lib/pdf-liteparse.ts`'s batch-vs-whole-document decision reuses
  // this instead of re-opening the PDF to count pages a second time.
  const pdfPageCountForFileRef = useRef<WeakMap<File, number>>(new WeakMap());

  useEffect(() => {
    let cancelled = false;

    // Fresh-start: ?reset=1 clears all IndexedDB, Cache API, localStorage, sessionStorage,
    // and forces a hard reload to see first-user asset download flow.
    const params = new URLSearchParams(window.location.search);
    if (params.get("reset") === "1") {
      (async () => {
        try {
          const dbs = await indexedDB.databases?.();
          if (dbs && dbs.length) {
            await Promise.all(
              dbs.map((db) => {
                return new Promise<void>((resolve) => {
                  const req = indexedDB.deleteDatabase(db.name || "");
                  req.onsuccess = () => resolve();
                  req.onerror = () => resolve();
                  req.onblocked = () => resolve();
                });
              })
            );
            console.log("[AppState] Reset: cleared IndexedDB databases", dbs.map(d => d.name));
          }
          const g: any = globalThis as any;
          if (typeof g.caches !== "undefined" && g.caches) {
            const names = await g.caches.keys();
            await Promise.all(names.map((n: string) => g.caches.delete(n)));
            console.log("[AppState] Reset: cleared Cache API", names);
          }
          localStorage.clear();
          sessionStorage.clear();
          const url = new URL(window.location.href);
          url.searchParams.delete("reset");
          window.location.replace(url.toString());
        } catch (e) {
          console.warn("[AppState] Reset cleanup failed", e);
        }
      })();
      return;
    }

    async function resolveNerModel() {
      const deviceTier = detectDeviceTier();
      if (deviceTier === "low") {
        console.log("[AppState] low device-memory tier — skipping neural NER model");
        setNerModelDegraded(true);
        setError(
          "La mémoire disponible sur cet appareil est limitée — utilisation de la détection PII par regex uniquement, sans le modèle neuronal.",
        );
        setAssets((a) => ({ ...a, nerModel: true }));
        return;
      }
      if (await isModelCached()) {
        setAssets((a) => ({ ...a, nerModel: true }));
        return;
      }
      try {
        const load = await loadNerModel((p) => setNerModelProgress(p));
        if (!load.ok) {
          console.warn("[AppState] NER model not loadable:", load.reason, load.message);
          setNerModelDegraded(true);
          setError(load.message);
        }
        setAssets((a) => ({ ...a, nerModel: true }));
        setNerModelProgress(null);
      } catch (e) {
        console.warn("[AppState] NER model download failed, using fallback:", e);
        setNerModelDegraded(true);
        setError("Backend neuronal PII indisponible — repli sur la détection par regex uniquement.");
        setAssets((a) => ({ ...a, nerModel: true }));
        setNerModelProgress(null);
      }
    }

    async function preloadAssets() {
      try {
        console.log("[AppState] preloadAssets started");
        setAssets((a) => ({ ...a, xbergWasm: true }));
        await preloadXbergWasm();
        console.log("[AppState] preloadXbergWasm done");

        await resolveNerModel();

        try {
          await loadTessdata("eng");
        } catch (e) {
          console.warn("[AppState] Tesseract tessdata download failed, OCR unavailable:", e);
        }
        setAssets((a) => ({ ...a, tessdata: true }));
        localStorage.setItem("xberg-studio-visited", "true");
      } catch (e) {
        console.error("[AppState] preloadAssets error:", e);
        setNerModelDegraded(true);
        setError(
          "Échec du chargement des modèles — détection PII neuronale indisponible, repli sur la détection par regex uniquement.",
        );
        setAssets({ xbergWasm: true, nerModel: true, tessdata: true });
        localStorage.setItem("xberg-studio-visited", "true");
      }
    }

    async function init() {
      const visited = localStorage.getItem("xberg-studio-visited");
      if (visited) {
        setAssets((a) => ({ ...a, xbergWasm: true, tessdata: true }));
        await resolveNerModel();
      } else {
        await preloadAssets();
      }
      if (cancelled) return;

      const pool = await createWorkerPool(
        { poolSize: poolSizeForTier(detectDeviceTier()) },
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

      pool.setProgressHandler((update) => {
        setProgress((prev) => new Map(prev).set(update.file, update));
      });
      pool.setFileCompleteHandler((result) => {
        setResults((prev) => [...prev, result]);
        setProgress((prev) =>
          new Map(prev).set(result.frontmatter.source, {
            file: result.frontmatter.source,
            stage: "complete",
            percent: 100,
          })
        );
        void saveDocument(result);
      });
      pool.setErrorHandler((file, error) => {
        setError(`${file}: ${error}`);
        setFileErrors((prev) => new Map(prev).set(file, error));
      });
      pool.setWarningHandler((message) => {
        console.warn("[WorkerPool] Warning:", message);
        toast.warning(message);
      });
      pool.setBatchCompleteHandler(() => {
        setTimeout(() => {
          setProgress(new Map());
        }, 1000);
      });

      setWorkerReady(true);
    }

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
          const { WhisperBridge } = await import("./transcription/whisper-bridge");
          whisperBridgeRef.current = new WhisperBridge();
        }
        const result: TranscriptionResult = await whisperBridgeRef.current.transcribeAudio(
          data.audioBytes,
          data.mimeType,
          data.config,
          (phase, fraction) => {
            const percent =
              phase === "resample" ? 10 + fraction * 30 : 40 + fraction * 50;
            setProgress((prev) =>
              new Map(prev).set(data.file, {
                file: data.file,
                stage: "transcribe",
                percent,
                message:
                  phase === "resample"
                    ? `Rééchantillonnage audio (${Math.round(fraction * 100)}%)`
                    : `Transcription (${Math.round(fraction * 100)}%)`,
              }),
            );
          },
        );
        respond({ result });
      } catch (e) {
        respond({ error: e instanceof Error ? e.message : "Erreur inconnue" });
      }
    }

    init();
    return () => {
      cancelled = true;
      workerPoolRef.current?.terminate();
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps -- runs once, mirrors onMount
  }, []);

  useEffect(() => {
    let cancelled = false;
    async function hydrate() {
      let persistedDocs: Awaited<ReturnType<typeof listDocuments>>;
      let persistedEdits: Awaited<ReturnType<typeof listEditedFindings>>;
      try {
        [persistedDocs, persistedEdits] = await Promise.all([
          listDocuments(),
          listEditedFindings(),
        ]);
      } catch (e) {
        console.warn("[AppState] Failed to hydrate document library from IndexedDB:", e);
        return;
      }
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
    }
    hydrate();
    return () => {
      cancelled = true;
    };
  }, []);

  useEffect(() => {
    void pruneDrafts(30 * 24 * 60 * 60 * 1000);
  }, []);

  async function handleFilesAccepted(fileList: FileList | File[]): Promise<void> {
    console.log("[AppState] handleFilesAccepted called with", fileList.length, "files");
    const fileArray = Array.from(fileList);
    const validFiles: File[] = [];
    let skippedJunk = 0;
    const unsupported: string[] = [];
    const pageSafetyWarnings: string[] = [];

    for (const file of fileArray) {
      const name = effectiveFileName(file);
      if (isJunkFile(name)) {
        skippedJunk++;
        continue;
      }
      const validation = validateFile(file);
      if (!validation.valid) {
        unsupported.push(validation.error || "Fichier invalide");
        console.warn("[AppState] skipping unsupported file:", name, validation.error);
        continue;
      }

      const pageSafety = await checkPdfPageSafety(file);
      if (!pageSafety.valid) {
        unsupported.push(pageSafety.error || "PDF rejeté");
        console.warn("[AppState] skipping PDF over page-count limit:", name, pageSafety.error);
        continue;
      }
      if (pageSafety.disableOcr) {
        disableOcrForFileRef.current.set(file, true);
        if (pageSafety.warning) pageSafetyWarnings.push(pageSafety.warning);
      }
      if (pageSafety.pageCount !== undefined) {
        pdfPageCountForFileRef.current.set(file, pageSafety.pageCount);
      }

      validFiles.push(file);
    }

    const totalSkipped = unsupported.length + skippedJunk;
    if (fileArray.length === 1 && unsupported.length === 1) {
      setError(unsupported[0]);
      setSkipNotice(null);
    } else if (totalSkipped > 0) {
      const skipParts: string[] = [];
      if (unsupported.length > 0) skipParts.push(`${unsupported.length} non pris en charge`);
      if (skippedJunk > 0) skipParts.push(`${skippedJunk} système`);
      setSkipNotice(`${totalSkipped === 1 ? "Fichier ignoré" : "Fichiers ignorés"} : ${skipParts.join(" et ")}.`);
    } else {
      setSkipNotice(null);
    }

    if (pageSafetyWarnings.length > 0) {
      setSkipNotice((prev) => {
        const combined = [prev, ...pageSafetyWarnings].filter(Boolean).join(" ");
        return combined || null;
      });
    }

    if (validFiles.length === 0) return;
    setPendingFiles((prev) => [...prev, ...validFiles]);
    setError(null);
  }

  function handleRemovePending(index: number): void {
    setPendingFiles((prev) => prev.filter((_, i) => i !== index));
  }

  function handleClearPending(): void {
    setPendingFiles([]);
  }

  async function handleProcessQueue(): Promise<void> {
    const queued = pendingFiles;
    if (queued.length === 0) return;
    setPendingFiles([]);
    setFiles((prev) => [...prev, ...queued]);

    const fileInputs = await Promise.all(
      queued.map(async (f) => ({
        name: effectiveFileName(f),
        bytes: await f.arrayBuffer(),
        type: f.type || "application/octet-stream",
        disableOcr: disableOcrForFileRef.current.get(f),
        pdfPageCount: pdfPageCountForFileRef.current.get(f),
      })),
    );

    console.log("[AppState] posting to worker pool:", fileInputs.length, "files");
    const pool = workerPoolRef.current;
    if (!pool) {
      setError("Le pool de workers n'est pas initialisé");
      return;
    }

    try {
      const batchConfig = JSON.parse(JSON.stringify(configRef.current)) as AppConfig;
      const results = await pool.processFiles(fileInputs, batchConfig);
      console.log("[AppState] Worker pool completed:", results.length, "files");

      if (
        batchConfig.enablePiiDetection &&
        batchConfig.redactPiiInOutput &&
        batchConfig.pseudonymPassphrase &&
        (batchConfig.redactionMode === "pseudonymize" || batchConfig.redactionMode === "hash") &&
        results.length > 0
      ) {
        recordKeyUsage(batchConfig.pseudonymKeyId);
      }
    } catch (error) {
      console.error("[AppState] Worker pool error:", error);
      const message = error instanceof Error ? error.message : "Échec du traitement par le pool de workers";
      setError(message);
      setFileErrors((prev) => {
        const next = new Map(prev);
        for (const input of fileInputs) {
          if (progressRef.current.get(input.name)?.stage !== "complete") {
            next.set(input.name, message);
          }
        }
        return next;
      });
    }
  }

  function toggleFolderMode(event: React.MouseEvent): void {
    event.preventDefault();
    setFolderMode((v) => !v);
  }

  function findingsFor(result: ProcessedFile): PiiEntity[] {
    return editedFindings.get(result.name) ?? result.piiFindings;
  }

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

  function handleRemoveFinding(result: ProcessedFile, index: number): void {
    setEditedFindings((prev) => {
      const current = prev.get(result.name) ?? result.piiFindings;
      const next = current.filter((_, i) => i !== index);
      void saveEditedFindings(result.name, next);
      return new Map(prev).set(result.name, next);
    });
  }

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

  const libraryDocuments: LibraryDocument[] = results.map((result) => ({
    result,
    findings: findingsFor(result),
    folder: folderOf(result.name),
    baseName: result.name,
  }));

  function findResultByName(name: string): ProcessedFile | undefined {
    return results.find((r) => r.name === name);
  }

  function findOriginalFile(result: ProcessedFile): File | undefined {
    return files.find((f) => effectiveFileName(f) === result.frontmatter.source);
  }

  // ---- Task 3: session wiring (Détection / Traitement / Conversion) ----
  const detectionSelection: ReadonlySet<string> = session.detectionSelection;
  const treatmentMode: TreatmentMode = session.treatmentMode;
  const conversionMode: ConversionMode = session.conversionMode;

  function setDetectionSelection(next: ReadonlySet<string>): void {
    const nextSet = new Set(next);
    setSession((prev) => ({ ...prev, detectionSelection: nextSet }));
    const nerCategories: NerCategory[] = [];
    const nerCustomLabels: string[] = [];
    for (const code of next) {
      const wire = categoryToWire(code);
      if (wire === undefined) continue;
      if (typeof wire === "string") {
        if (isNerCategory(wire)) nerCategories.push(wire);
        else nerCustomLabels.push(wire);
      } else {
        nerCustomLabels.push(wire.custom);
      }
    }
    setConfig((prev) => ({ ...prev, nerCategories, nerCustomLabels }));
  }

  function setTreatmentMode(mode: TreatmentMode): void {
    setSession((prev) => ({ ...prev, treatmentMode: mode }));
    if (mode === "pseudonymize") {
      setConfig((prev) => ({ ...prev, redactionMode: "pseudonymize", redactPiiInOutput: true, enablePiiDetection: true }));
    } else {
      setConfig((prev) => ({ ...prev, redactionMode: "mask", redactPiiInOutput: true, enablePiiDetection: true }));
    }
  }

  function setConversionMode(mode: ConversionMode): void {
    setSession((prev) => ({ ...prev, conversionMode: mode }));
    setConfig((prev) => ({ ...prev, outputFormat: mode === "markdown" ? "markdown" : "plain" }));
  }

  function openDetectionModal(): void {
    toast.info("Sélection des données — à venir");
  }

  // `files` is append-only (only ever pushed onto, in `handleProcessQueue` above) — a
  // naive `files.length > 0` stays true forever after the very first batch. Derived
  // from actual in-flight work instead: a file with no progress entry yet (just
  // queued) or one whose stage isn't "complete"/"error" is still processing. Shared by
  // both `Studio.tsx` (whether to show the "Traitement" section) and `routes/__root.tsx`
  // (whether the sidebar should stay hidden) so they can't disagree.
  const isProcessing = files.some((f) => {
    const stage = progress.get(effectiveFileName(f))?.stage;
    return stage !== "complete" && stage !== "error";
  });

  return {
    assets,
    nerModelProgress,
    nerModelDegraded,
    files,
    pendingFiles,
    progress,
    isProcessing,
    results,
    config,
    setConfig,
    error,
    setError,
    folderMode,
    skipNotice,
    setSkipNotice,
    workerReady,
    fileErrors,
    editedFindings,
    libraryDocuments,
    handleFilesAccepted,
    handleRemovePending,
    handleClearPending,
    handleProcessQueue,
    toggleFolderMode,
    findingsFor,
    handleAddFinding,
    handleRemoveFinding,
    handleDeleteDocuments,
    findResultByName,
    findOriginalFile,
    exportBody: (result: ProcessedFile, body: string) => downloadText(result.name, wrapRedactedBody(result, body)),
    downloadZip,
    session,
    detectionSelection,
    treatmentMode,
    conversionMode,
    setDetectionSelection,
    setTreatmentMode,
    setConversionMode,
    openDetectionModal,
  };
}

type AppState = ReturnType<typeof useAppStateProvider>;

const AppStateContext = createContext<AppState | null>(null);

export function AppStateProvider({ children }: { children: ReactNode }) {
  const state = useAppStateProvider();
  return <AppStateContext.Provider value={state}>{children}</AppStateContext.Provider>;
}

export function useAppState(): AppState {
  const ctx = useContext(AppStateContext);
  if (!ctx) throw new Error("useAppState must be used within an AppStateProvider");
  return ctx;
}
