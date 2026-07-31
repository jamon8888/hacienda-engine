import { useEffect, useRef, useState } from "react";
import { Onboarding } from "./components/Onboarding";
import { ConfigPanel } from "./components/ConfigPanel";
import { ProgressBar } from "./components/ProgressBar";
import { PiiPanel } from "./components/PiiPanel";
import { MarkdownEditor } from "./components/MarkdownEditor";
import { loadNerModel, isModelCached, preloadXbergWasm, validateFile } from "./lib/asset-loader";
import { effectiveFileName, isJunkFile } from "./lib/file-filter";
import { DEFAULT_CONFIG } from "./lib/types";
import type { AppConfig, OnboardingState, ProcessedFile, ProgressUpdate } from "./lib/types";

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

  const workerRef = useRef<Worker | null>(null);
  const configRef = useRef(config);
  configRef.current = config;
  const fileInputRef = useRef<HTMLInputElement>(null);

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
          setTimeout(() => {
            setProgress(new Map());
            setFiles([]);
          }, 1000);
          break;
        case "error":
          setError(`${data.file}: ${data.message}`);
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

  function onDrop(event: React.DragEvent): void {
    event.preventDefault();
    if (!workerReady) return;
    if (event.dataTransfer?.files) handleFiles(event.dataTransfer.files);
  }

  function onDragOver(event: React.DragEvent): void {
    event.preventDefault();
    event.dataTransfer.dropEffect = "copy";
  }

  function toggleFolderMode(event: React.MouseEvent): void {
    event.preventDefault();
    setFolderMode((v) => !v);
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
        <section
          className="drop-zone cursor-pointer rounded-xl border-2 border-dashed border-border bg-card p-16 text-center transition-colors hover:border-primary"
          role="group"
          aria-label="Upload documents"
          onDrop={onDrop}
          onDragOver={onDragOver}
        >
          <input
            ref={fileInputRef}
            type="file"
            id="file-input"
            multiple
            disabled={!workerReady}
            accept={folderMode ? undefined : UPLOAD_ACCEPT}
            className="absolute h-px w-px overflow-hidden opacity-0"
            style={{ zIndex: -1 }}
            aria-label={folderMode ? "Choose a folder" : "Choose files"}
            onChange={(e) => {
              const list = e.target.files;
              if (list) handleFiles(list);
              e.target.value = "";
            }}
            {...(folderMode ? { webkitdirectory: "" } : {})}
          />
          <label htmlFor="file-input" className="block cursor-pointer">
            <span aria-hidden="true" className="mb-2 block text-5xl">
              {folderMode ? "📁" : "📄"}
            </span>
            <p className="mb-1 text-muted-foreground">
              {workerReady
                ? folderMode
                  ? "Drop a folder here or click to browse"
                  : "Drop files here or click to browse"
                : "Starting the local engine…"}
            </p>
            <p className="text-sm text-muted-foreground">
              PDF, Office, Email, Images, Audio/Video, Subtitles, Code — up to 50MB each
            </p>
          </label>
          <button
            type="button"
            className="mode-toggle mx-auto mt-4 block bg-transparent text-xs text-muted-foreground underline hover:text-primary disabled:cursor-not-allowed disabled:opacity-60"
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

        {results.length > 0 && (
          <section className="mt-8 flex flex-col gap-4 border-t border-border pt-6">
            <h2 className="text-sm font-semibold text-muted-foreground">
              Processed this batch ({results.length})
            </h2>
            {results.map((result) => (
              <div key={result.name} className="flex flex-col gap-2">
                <div className="flex items-center justify-between text-sm">
                  <span className="font-medium">{result.name}</span>
                  <span className="text-muted-foreground">
                    {result.entities.length}{" "}
                    {result.entities.length === 1 ? "entity" : "entities"}
                  </span>
                </div>
                <MarkdownEditor value={result.rawMarkdown} findings={result.piiFindings} />
                {result.piiFindings.length > 0 && <PiiPanel findings={result.piiFindings} />}
              </div>
            ))}
          </section>
        )}
      </main>

      {showConfig && (
        <ConfigPanel config={config} onChange={setConfig} onDone={() => setShowConfig(false)} />
      )}
    </div>
  );
}
