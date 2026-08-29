import { Archive, Trash2, Play, Check, Loader2, Settings2, Lock } from "lucide-react";
import { FileUpload } from "@/components/extend/file-upload";
import { Button } from "@/components/ui/button";
import { Switch } from "@/components/ui/switch";
import { effectiveFileName } from "@/lib/file-filter";
import { exportDocumentsZip } from "@/lib/export-zip";
import type { AppConfig, OnboardingState, ProcessedFile, ProgressUpdate } from "@/lib/types";

const UPLOAD_ACCEPT =
  ".pdf,.docx,.xlsx,.pptx,.odt,.ods,.odp,.eml,.msg,.pst,.png,.jpg,.jpeg,.gif,.webp,.tiff,.bmp,.svg,.srt,.vtt,.txt,.md,.json,.csv,.xml,.html,.mp3,.wav,.m4a,.ogg,.flac,.aac,.mp4,.mov,.webm,.mkv";

// Screenshot shows 6 pipeline pills — keep order exactly as rendered
const STAGE_ORDER = ["extract", "ocr", "chunk", "ner", "pii", "redact"] as const;
type DisplayStage = (typeof STAGE_ORDER)[number];

const STAGE_LABELS: Record<DisplayStage, string> = {
  extract: "extract",
  ocr: "ocr",
  chunk: "chunk",
  ner: "ner",
  pii: "pii",
  redact: "redact",
};

function mapToDisplayIndex(update: ProgressUpdate | undefined): number {
  if (!update) return -1;
  switch (update.stage) {
    case "queued":
    case "wasm-load":
    case "error":
      return -1;
    case "extract":
      // 10% start → extract active, 50% finish → extract done, ocr/chunk will be marked done on next stage
      return 0;
    case "transcribe":
      // transcription runs where OCR would for audio/video — light up ocr pill
      return 1;
    case "ner":
      return 3;
    case "pii":
      return 4;
    case "link":
      return 5;
    case "complete":
      return STAGE_ORDER.length; // all done
    default:
      return -1;
  }
}

function StagePills({ update }: { update: ProgressUpdate | undefined }) {
  const activeIndex = mapToDisplayIndex(update);
  const isComplete = update?.stage === "complete";
  return (
    <div className="flex flex-wrap gap-1.5">
      {STAGE_ORDER.map((stage, i) => {
        const done = isComplete || (activeIndex >= 0 && i < activeIndex);
        const isDone = done || (activeIndex >= 3 && i < 3);
        const active = !isComplete && i === activeIndex;
        return (
          <span
            key={stage}
            className={
              "rounded border px-1.5 py-0.5 font-mono text-[10px] leading-none tracking-wide " +
              (isDone
                ? "border-emerald-500/30 bg-emerald-500/10 text-emerald-400"
                : active
                  ? "border-primary/40 bg-primary/15 text-primary"
                  : "border-border bg-muted/40 text-muted-foreground/50")
            }
          >
            {STAGE_LABELS[stage]}
          </span>
        );
      })}
    </div>
  );
}

function formatBytes(bytes: number): string {
  return bytes < 1024 * 1024
    ? `${Math.max(1, Math.round(bytes / 1024))} KB`
    : `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}

export function Studio({
  workerReady,
  folderMode,
  onToggleFolderMode,
  onFilesAccepted,
  pendingFiles,
  onRemovePending,
  onClearPending,
  onProcessQueue,
  files,
  progress,
  results,
  fileErrors,
  onOpenDocument,
  config,
  onConfigChange,
  onOpenSettings,
  assets,
  nerModelProgress,
  nerModelDegraded,
  isProcessing,
}: {
  workerReady: boolean;
  folderMode: boolean;
  onToggleFolderMode: (e: React.MouseEvent) => void;
  onFilesAccepted: (files: File[]) => void;
  pendingFiles: File[];
  onRemovePending: (index: number) => void;
  onClearPending: () => void;
  onProcessQueue: () => void;
  files: File[];
  progress: Map<string, ProgressUpdate>;
  results: ProcessedFile[];
  fileErrors: Map<string, string>;
  onOpenDocument: (name: string) => void;
  config: AppConfig;
  onConfigChange: (c: AppConfig) => void;
  onOpenSettings: () => void;
  assets: OnboardingState["assets"];
  nerModelProgress: { receivedBytes: number; totalBytes: number | null } | null;
  nerModelDegraded: boolean;
  isProcessing: boolean;
}) {
  const resultsByInput = new Map(results.map((r) => [r.frontmatter.source, r] as const));
  const processedCount = files.filter((f) => progress.get(effectiveFileName(f))?.stage === "complete").length;
  const totalCount = files.length;

  const assetsReady = assets.xbergWasm && assets.nerModel && assets.tessdata;

  return (
    <div className="flex flex-1 flex-col bg-background text-foreground">
      {/* ── Section 1: the whole upload block, hero copy included — this is the
          homepage's first section, and its H1 is that section's title. ── */}
      <section className="border-b border-border px-4 py-10 sm:px-6 sm:py-14">
        <div className="mx-auto flex w-full max-w-[720px] flex-col">
          <div className="flex flex-wrap items-center justify-between gap-3">
            <span className="inline-flex items-center gap-2 rounded-full border border-border bg-card px-3 py-1 text-xs text-muted-foreground">
              <Lock className="size-3" /> Pipeline local · zéro upload
            </span>
            <div className="flex items-center gap-2">
               {results.length > 0 && (
                 <Button
                   size="sm"
                   variant="outline"
                   className="h-8"
                   onClick={() => exportDocumentsZip(results.map((result) => ({ result })))}
                 >
                   <Archive className="size-3.5" /> Télécharger
                 </Button>
               )}
              <Button size="sm" variant="outline" className="h-8" onClick={onOpenSettings}>
                <Settings2 className="size-3.5" /> Paramètres
              </Button>
            </div>
          </div>

          {/* Asset loading progress on first visit — single bar */}
          {!assetsReady && (() => {
            let label = "Préparation du workspace";
            let pct = 0;
            if (!assets.xbergWasm) {
              label = "Chargement du runtime…";
              pct = 0;
            } else if (!assets.nerModel) {
              label = nerModelDegraded ? "Modèle neuronal indisponible — repli sur regex" : "Téléchargement du modèle d'entités…";
              if (nerModelProgress?.totalBytes) {
                pct = Math.min(100, Math.round((nerModelProgress.receivedBytes / nerModelProgress.totalBytes) * 100));
              } else if (!nerModelDegraded) {
                pct = 10; // indeterminate start
              }
            } else if (!assets.tessdata) {
              label = "Téléchargement des données OCR…";
              pct = 80;
            } else {
              label = "Prêt";
              pct = 100;
            }
            return (
              <div className="mt-4 rounded-md border border-border bg-card/50 px-3 py-2">
                <div className="flex items-center justify-between text-xs">
                  <span className="text-muted-foreground">{label}</span>
                  <span className="text-muted-foreground">{pct}%</span>
                </div>
                <div className="mt-1 h-1.5 overflow-hidden rounded-full bg-muted">
                  <div className="h-full bg-primary transition-[width] duration-300" style={{ width: `${pct}%` }} />
                </div>
                <p className="mt-1 text-[11px] text-muted-foreground">
                  Runtime {assets.xbergWasm ? "✓" : "…"} · Modèle {assets.nerModel ? (nerModelDegraded ? "dégradé" : "✓") : "…"} · OCR {assets.tessdata ? "✓" : "…"}
                </p>
              </div>
            );
          })()}

          <h1 className="mt-6 font-display text-4xl font-semibold leading-tight tracking-tight sm:text-5xl">
            Masquez les documents sensibles sans qu'ils quittent votre ordinateur.
          </h1>
          <p className="mt-4 max-w-xl text-base leading-relaxed text-muted-foreground">
            Déposez des fichiers ci-dessous, revoyez ce que le pipeline a trouvé, corrigez-le et
            exportez des copies propres avec une piste d'audit — tout dans cet onglet du navigateur.
          </p>
          <div className="mt-6 flex flex-wrap items-center gap-2 text-xs text-muted-foreground">
            {["Extraire", "Reconnaître", "Découper", "Entités", "PII", "Masquer"].map(
              (step, i, arr) => (
                <span key={step} className="flex items-center gap-2">
                  <span className="rounded-md border border-border px-2 py-1 font-mono">
                    {step}
                  </span>
                  {i < arr.length - 1 && <span>→</span>}
                </span>
              ),
            )}
          </div>
        </div>
      </section>

      <div className="mx-auto flex w-full max-w-[720px] flex-col px-4 py-6 sm:px-6 sm:py-8">
        {/* ── Upload — always visible, stays on the same page as progress ── */}
        <div className="flex flex-col">

          <div className="mb-4 flex items-center justify-between gap-3 rounded-lg border border-border bg-card px-3.5 py-2.5">
            <div className="min-w-0">
              <p className="text-sm font-medium">Rédiger les PII dans la sortie</p>
              <p className="mt-0.5 text-xs text-muted-foreground">
                {!config.enablePiiDetection
                  ? "Nécessite la reconnaissance d'entités (désactivée dans les Paramètres) — sans elle, aucune PII n'est détectée à rédiger."
                  : config.redactPiiInOutput
                    ? "Les portions détectées sont masquées dans les documents exportés."
                    : "Les PII détectées sont comptées mais laissées telles quelles dans la sortie."}
              </p>
            </div>
            <Switch
              aria-label="Rédiger les PII dans la sortie"
              checked={config.redactPiiInOutput}
              disabled={!config.enablePiiDetection}
              onCheckedChange={(checked) => onConfigChange({ ...config, redactPiiInOutput: checked })}
            />
          </div>

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
            inputAriaLabel={folderMode ? "Choisir un dossier" : "Choisir des fichiers"}
            title={
              workerReady
                ? folderMode
                  ? "Déposez un dossier ici ou cliquez pour parcourir"
                  : "Déposez des fichiers ici ou cliquez pour parcourir"
                : "Démarrage du moteur local…"
            }
            description="PDF, Office, E-mail, Images, Audio/Vidéo, Sous-titres, Code — jusqu'à 50 Mo chacun"
            onFilesAccepted={onFilesAccepted}
          />
           <button
             type="button"
             className="mode-toggle mx-auto mt-3 block bg-transparent text-xs text-muted-foreground underline decoration-border underline-offset-2 hover:text-foreground disabled:cursor-not-allowed disabled:opacity-60"
             disabled={!workerReady}
             onClick={onToggleFolderMode}
           >
             {folderMode ? "ou choisir des fichiers individuels" : "ou choisir un dossier"}
           </button>

          {pendingFiles.length > 0 && (
            <section aria-labelledby="studio-pending-heading" className="mt-6 overflow-hidden rounded-xl border border-border bg-card">
              <h2 id="studio-pending-heading" className="sr-only">Fichiers sélectionnés</h2>
              <ul>
                {pendingFiles.map((file, i) => (
                  <li
                    key={`${effectiveFileName(file)}-${i}`}
                    className="flex items-center justify-between gap-3 border-b border-border/60 px-4 py-3 last:border-b-0"
                  >
                    <span className="truncate font-mono text-xs text-foreground/80">{effectiveFileName(file)}</span>
                    <div className="flex items-center gap-2">
                      <span className="font-mono text-[11px] text-muted-foreground">{formatBytes(file.size)}</span>
                      <button
                        type="button"
                        aria-label={`Retirer ${effectiveFileName(file)}`}
                        className="inline-flex size-7 items-center justify-center rounded-full bg-muted text-muted-foreground hover:bg-destructive/15 hover:text-destructive"
                        onClick={() => onRemovePending(i)}
                      >
                        <Trash2 className="size-3.5" />
                      </button>
                    </div>
                  </li>
                ))}
              </ul>
               <div className="flex items-center justify-between bg-muted/40 px-4 py-3">
                 <button type="button" className="text-xs text-muted-foreground hover:text-foreground" onClick={onClearPending}>
                   Tout effacer
                 </button>
                 <Button size="sm" className="h-8 px-4 text-xs" onClick={onProcessQueue}>
                   <Play className="size-3.5" /> Traiter {pendingFiles.length} fichier{pendingFiles.length === 1 ? "" : "s"}
                 </Button>
               </div>
            </section>
          )}
        </div>

         {/* ── Processing — stays in the SAME upload page, exactly like the screenshot ── */}
         {isProcessing && (
           <div className="mt-8 flex flex-col">
             <div className="mb-4">
               <h2 className="text-base font-semibold tracking-tight">Traitement</h2>
               <p className="mt-1 text-xs text-muted-foreground">
                 {processedCount} sur {totalCount} terminés · chaque fichier passe par le pipeline complet indépendamment.
               </p>
             </div>

            <div className="flex flex-col gap-2.5" aria-live="polite" aria-label="File d'attente de traitement">
              {files.map((file) => {
                const key = effectiveFileName(file);
                const update = progress.get(key);
                const result = resultsByInput.get(key);
                const error = fileErrors.get(key);
                const percent = update?.percent ?? (error ? 0 : 0);
                const isComplete = update?.stage === "complete";
                const isQueued = !update || update.stage === "queued" || update.stage === "wasm-load";
                const isError = !!error || update?.stage === "error";
                const statusLabel = isError ? "Échec" : isComplete ? "Terminé" : isQueued ? "En file d'attente" : update?.message || stageToHuman(update?.stage);
                const displayPercent = isComplete ? 100 : isQueued ? 0 : Math.round(percent);
                return (
                  <div
                    key={key}
                    className={
                      "rounded-lg border bg-card p-3.5 " +
                      (isError ? "border-destructive/30" : "border-border")
                    }
                  >
                    <div className="flex items-center gap-2.5">
                      <span
                        className={
                          "flex size-3.5 shrink-0 items-center justify-center rounded-full text-[10px] " +
                          (isComplete
                            ? "bg-emerald-500/15 text-emerald-400 ring-1 ring-emerald-500/30"
                            : isError
                              ? "bg-destructive/15 text-destructive ring-1 ring-destructive/20"
                              : isQueued
                                ? "bg-muted text-muted-foreground/50 ring-1 ring-border"
                                : "bg-primary/15 text-primary ring-1 ring-primary/30")
                        }
                        aria-hidden
                      >
                        {isComplete ? <Check className="size-2.5" strokeWidth={3} /> : isQueued ? <span className="size-1.5 rounded-full bg-muted-foreground/40" /> : <Loader2 className="size-2.5 animate-spin" />}
                      </span>
                      <button
                        type="button"
                        className="min-w-0 flex-1 truncate text-left font-mono text-xs font-medium text-foreground/90 hover:text-foreground disabled:cursor-default disabled:text-foreground/90"
                        disabled={!result || isError}
                        onClick={() => result && onOpenDocument(result.name)}
                        title={key}
                      >
                        {key}
                      </button>
                      <span className="shrink-0 font-mono text-[11px] tabular-nums text-muted-foreground">{displayPercent}%</span>
                    </div>

                    <div className="mt-3 h-1 overflow-hidden rounded-full bg-muted">
                      <div
                        className="h-full rounded-full bg-primary transition-[width] duration-500 ease-out"
                        style={{ width: `${displayPercent}%` }}
                      />
                    </div>

                    <div className="mt-2.5">
                      <StagePills update={update} />
                    </div>
                    <p className="mt-1.5 truncate font-mono text-[10px] leading-none text-muted-foreground/70">{statusLabel}</p>
                    {isError && <p className="mt-2 break-words text-xs leading-relaxed text-destructive">{error}</p>}
                  </div>
                );
              })}
            </div>
          </div>
        )}

        {pendingFiles.length > 0 && isProcessing && (
          <div className="mt-3 rounded-lg border border-primary/15 bg-primary/[0.06] px-4 py-3 text-xs text-primary/80">
            {pendingFiles.length} autre(s) fichier(s) en file d'attente de révision — pas encore envoyés au pipeline.
          </div>
        )}
      </div>
    </div>
  );
}

function stageToHuman(stage: ProgressUpdate["stage"] | undefined): string {
  switch (stage) {
    case "extract":
      return "Extraction du contenu";
    case "transcribe":
      return "Transcription audio";
    case "ner":
      return "Notation des entités";
    case "pii":
      return "Détection des PII";
    case "link":
      return "Mise en forme & liaison";
    default:
      return "Traitement";
  }
}
