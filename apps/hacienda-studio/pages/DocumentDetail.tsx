import { useEffect, useMemo, useRef, useState, Suspense, lazy } from "react";
import {
  ArrowLeft,
  Download,
  Trash2,
  ShieldCheck,
  FileText,
  ScanSearch,
  LayoutGrid,
  ClipboardCheck,
} from "lucide-react";
import { toast } from "sonner";
import { Button } from "@/components/ui/button";
import { CodeLines } from "@/components/CodeLines";
import { DocumentOutline } from "@/components/DocumentOutline";
import { MarkdownEditor } from "@/components/MarkdownEditor";
import { InteractiveEditor } from "@/components/editor/InteractiveEditor";
import { PiiPanel } from "@/components/PiiPanel";
import { ViewerErrorBoundary } from "@/components/ViewerErrorBoundary";
import { RedactedEditor } from "@/components/RedactedEditor";
// Lazy, not a static import: `@extend-ai/react-{docx,pptx,xlsx}` each ship their own
// import-workers and CJS/UMD dependencies (utif, pako, regl — see vite.config.ts's
// optimizeDeps comment) that a bad interaction with Vite's dep optimizer can throw on at
// module-evaluation time. A static import here means that failure crashes the *whole*
// app on first load (any document, not just docx/xlsx/pptx ones) — same reasoning as
// App.tsx's own lazy `Documents` import.
const DocxViewerPreview = lazy(() =>
  import("@/components/extend/docx-viewer").then((m) => ({ default: m.DocxViewerPreview })),
);
const XlsxViewerPreview = lazy(() =>
  import("@/components/extend/xlsx-viewer").then((m) => ({ default: m.XlsxViewerPreview })),
);
const PptxViewerPreview = lazy(() =>
  import("@/components/extend/pptx-viewer").then((m) => ({ default: m.PptxViewerPreview })),
);
import { PDFViewer } from "@/components/extend/pdf-viewer";
import { renderAnnotatedMarkdown } from "@/lib/annotate";
import { getViewerKind } from "@/lib/viewer-kind";
import { computeContentHash } from "@/lib/content-hash";
import { loadDraft, saveDraft } from "@/lib/redaction-store";
import { getAuditChainTip, listAuditEntries, verifyAuditChain, type AuditEntryRow } from "@/lib/pii-engine";
import {
  REDACTION_MODES,
  applyRedactionMode,
  type RedactionMode,
} from "@/lib/redaction-modes";
import type { ProcessedFile } from "@/lib/types";
import type { PiiEntity } from "@/lib/pii-engine";

type DetailTab = "redacted" | "source" | "findings" | "layout" | "audit";

export function DocumentDetail({
  result,
  findings,
  originalFile,
  onBack,
  onAddFinding,
  onRemoveFinding,
  onExportBody,
  onDelete,
}: {
  result: ProcessedFile;
  findings: PiiEntity[];
  originalFile: File | undefined;
  onBack: () => void;
  onAddFinding: (start: number, end: number, category: string) => void;
  onRemoveFinding: (index: number) => void;
  onExportBody: (body: string) => void;
  onDelete: () => void;
}) {
  const [mode, setMode] = useState<RedactionMode>("mask");
  const [previewUrl, setPreviewUrl] = useState<string | undefined>(undefined);
  const [contentHash, setContentHash] = useState<string | undefined>(undefined);
  const [draft, setDraft] = useState<string | undefined>(undefined);
  const [activeTab, setActiveTab] = useState<DetailTab>("redacted");

  const modeResult = useMemo(() => applyRedactionMode(findings, mode), [findings, mode]);
  const docPath = `documents/${result.name}`;
  const computedMarkdown =
    "findings" in modeResult
      ? renderAnnotatedMarkdown(result.rawMarkdown, result.entities, modeResult.findings, docPath)
      : null;

  const viewerKind = getViewerKind(result.frontmatter.source);
  const hasViewer = !!(viewerKind && previewUrl);

  const leftPanelRef = useRef<HTMLDivElement | null>(null);

  const manualUrlRef = useRef<string | null>(null);
  useEffect(() => {
    if (!originalFile) {
      // keep a manually re-added URL (Source tab re-add) — don't clobber it
      if (manualUrlRef.current) return;
      setPreviewUrl(undefined);
      return;
    }
    // original bytes became available again (fresh processing) — discard any manual URL
    if (manualUrlRef.current) {
      URL.revokeObjectURL(manualUrlRef.current);
      manualUrlRef.current = null;
    }
    const url = URL.createObjectURL(originalFile);
    setPreviewUrl(url);
    return () => URL.revokeObjectURL(url);
  }, [originalFile]);
  // Manual URL must be revoked on unmount — the effect above doesn't own it
  useEffect(() => {
    return () => {
      if (manualUrlRef.current) URL.revokeObjectURL(manualUrlRef.current);
    };
  }, []);

  useEffect(() => {
    setContentHash(undefined);
    setDraft(undefined);
    if (!originalFile) return;
    let cancelled = false;
    originalFile.arrayBuffer().then(async (bytes) => {
      const hash = await computeContentHash(bytes);
      if (!cancelled) setContentHash(hash);
    });
    return () => {
      cancelled = true;
    };
  }, [originalFile]);

  useEffect(() => {
    if (!contentHash) return;
    let cancelled = false;
    loadDraft(contentHash).then((saved) => {
      if (!cancelled && saved !== undefined) {
        // Toast only when the saved draft is actually adopted (`prev` was still unset).
        // If the user typed an edit before this resolved, `prev ?? saved` correctly
        // keeps their edit — but the toast previously fired unconditionally, telling
        // them a draft was restored even though their own edit was what's shown.
        setDraft((prev) => {
          if (prev === undefined) toast("Dernier brouillon masqué restauré pour ce fichier");
          return prev ?? saved;
        });
      }
    });
    return () => {
      cancelled = true;
    };
  }, [contentHash]);

  const pendingSaveRef = useRef<{ hash: string; draft: string } | null>(null);
  useEffect(() => {
    if (!contentHash || draft === undefined) return;
    pendingSaveRef.current = { hash: contentHash, draft };
    const timer = setTimeout(() => {
      pendingSaveRef.current = null;
      saveDraft(contentHash, draft)
        .then(() => toast.success("Brouillon masqué enregistré"))
        .catch(() => toast.error("Échec de l'enregistrement du brouillon masqué"));
    }, 1000);
    return () => clearTimeout(timer);
  }, [contentHash, draft]);
  useEffect(
    () => () => {
      const pending = pendingSaveRef.current;
      if (pending) void saveDraft(pending.hash, pending.draft);
    },
    [result.name],
  );

  function selectMode(next: RedactionMode) {
    setMode(next);
    const nextResult = applyRedactionMode(findings, next);
    if ("findings" in nextResult) {
      setDraft(renderAnnotatedMarkdown(result.rawMarkdown, result.entities, nextResult.findings, docPath));
    } else {
      // e.g. "pseudonymize" without reversible tokens available. Without this, `draft`
      // keeps the *previous* mode's rendered text, so `redactedBody` below stays defined
      // and the unavailable-mode message (keyed on `redactedBody === undefined`) never
      // renders — the pane silently shows mask/hash output while a mode that produced
      // nothing is the one selected.
      setDraft(undefined);
    }
  }

  const redactedBody = draft ?? computedMarkdown ?? undefined;

  const isPseudonymizeActive = mode === "pseudonymize";
  const modeButtonClass = (value: RedactionMode) =>
    value === mode
      ? value === "pseudonymize"
        ? "h-7 rounded-md border border-amber-500/50 bg-amber-500/15 px-3 text-xs font-medium text-amber-300 shadow-sm"
        : "h-7 rounded-md bg-primary px-3 text-xs font-medium text-primary-foreground shadow-sm"
      : "h-7 rounded-md border border-border bg-transparent px-3 text-xs font-medium text-muted-foreground hover:border-border hover:bg-accent hover:text-foreground";

  return (
    <div className="flex min-h-0 flex-1 flex-col bg-background text-foreground">
      {/* ── Top app bar is in App.tsx — this is the document sub-header ── */}
      <div className="flex items-center justify-between gap-4 border-b border-border bg-card px-4 py-2.5">
        <div className="flex min-w-0 items-center gap-2.5">
          <button
            type="button"
            onClick={onBack}
            className="inline-flex items-center gap-1.5 rounded-md px-2 py-1 text-xs font-medium text-muted-foreground hover:bg-accent hover:text-foreground"
          >
            <ArrowLeft className="size-3.5" />
            Documents
          </button>
          <span className="hidden h-4 w-px bg-border sm:block" aria-hidden />
          <span className="truncate text-sm font-semibold tracking-tight">{result.name}</span>
          <span className="inline-flex shrink-0 items-center rounded-full border border-border bg-background px-2 py-0.5 text-[11px] font-medium text-muted-foreground">
            {findings.length} détection{findings.length === 1 ? "" : "s"}
          </span>
        </div>

        <div className="flex shrink-0 items-center gap-1.5">
          {REDACTION_MODES.map((m) => (
            <button
              key={m.value}
              type="button"
              onClick={() => selectMode(m.value)}
              className={modeButtonClass(m.value)}
            >
              {m.label}
            </button>
          ))}
          <div className="ml-1 h-4 w-px bg-border" aria-hidden />
            <Button
              size="sm"
              className="h-7 gap-1.5 rounded-md bg-foreground px-3 text-xs font-medium text-background hover:bg-foreground/90"
              disabled={redactedBody === undefined}
              onClick={() => redactedBody !== undefined && onExportBody(redactedBody)}
            >
              <Download className="size-3.5" />
              Masqué
            </Button>
          <Button
            size="sm"
            variant="ghost"
            className="h-7 w-7 p-0 text-muted-foreground hover:bg-destructive/10 hover:text-destructive"
            onClick={onDelete}
            aria-label="Supprimer le document"
          >
            <Trash2 className="size-3.5" />
          </Button>
        </div>
      </div>

      {/* Missing-source banner */}
      {!originalFile && (
        <div className="flex items-start gap-2 border-b border-amber-500/20 bg-amber-500/[0.06] px-4 py-2.5 text-xs leading-relaxed text-amber-200/80">
          <span className="mt-0.5 inline-flex size-3.5 shrink-0 items-center justify-center rounded-full border border-amber-500/30 bg-amber-500/20 text-[10px] font-bold leading-none text-amber-400">
            !
          </span>
          <span>
            <span className="font-medium text-amber-300">Original non disponible dans cette session</span>
            <span className="text-amber-200/60"> — La sortie traitée est mise en cache localement, mais le fichier source n'est conservé qu'en mémoire. Ré-ajoutez le fichier pour voir l'aperçu natif.</span>
          </span>
        </div>
      )}

      {/* ── Split body: left document, right controls ── */}
      <div className="flex min-h-0 flex-1 flex-col lg:flex-row">
        {/* LEFT PANEL — document */}
        <div className="flex min-h-[420px] min-w-0 flex-1 flex-col border-b border-border bg-background lg:border-b-0 lg:border-r">
          {activeTab !== "layout" && (
            <DocumentOutline markdown={result.rawMarkdown} findings={findings} containerRef={leftPanelRef} />
          )}
          <div ref={leftPanelRef} className="flex-1 overflow-auto">
            {activeTab === "source" ? (
              hasViewer ? (
                <div className="h-full min-h-[520px] bg-background p-2">
                  <div className="h-full overflow-hidden rounded-lg border border-border bg-background">
                    <ViewerErrorBoundary fileName={result.frontmatter.source}>
                    <Suspense fallback={<ViewerLoadingFallback />}>
                      {(viewerKind === "docx" || viewerKind === "doc") && (
                        <DocxViewerPreview
                          src={previewUrl}
                          fileName={result.frontmatter.source}
                          isDark
                          showUpload={false}
                          onIsDarkChange={() => {}}
                        />
                      )}
                      {(viewerKind === "xlsx" || viewerKind === "xls") && (
                        <XlsxViewerPreview
                          src={previewUrl}
                          fileName={result.frontmatter.source}
                          isDark
                          showUpload={false}
                          onIsDarkChange={() => {}}
                        />
                      )}
                      {(viewerKind === "pptx" || viewerKind === "ppt") && (
                        <PptxViewerPreview src={previewUrl} fileName={result.frontmatter.source} showUpload={false} />
                      )}
                    </Suspense>
                    </ViewerErrorBoundary>
                    {viewerKind === "pdf" && (
                      <PDFViewer src={previewUrl} fileName={result.frontmatter.source} showUpload={false} />
                    )}
                  </div>
                </div>
              ) : viewerKind ? (
                <SourceReAddPrompt
                  viewerKind={viewerKind}
                  fileName={result.frontmatter.source}
                  onFilePicked={(file) => {
                    const url = URL.createObjectURL(file);
                    if (manualUrlRef.current) URL.revokeObjectURL(manualUrlRef.current);
                    manualUrlRef.current = url;
                    setPreviewUrl(url);
                  }}
                  fallbackMarkdown={result.rawMarkdown}
                />
              ) : (
                <div className="p-5">
                  <div className="mb-3 flex items-center gap-2 text-[11px] font-medium uppercase tracking-widest text-muted-foreground">
                    <FileText className="size-3.5" />
                    Markdown extrait
                    <span className="ml-auto rounded bg-muted px-1.5 py-0.5 font-mono text-[10px] normal-case tracking-normal text-muted-foreground">
                      markdown
                    </span>
                  </div>
                  <div className="rounded-lg border border-border bg-card">
                    <CodeLines text={result.rawMarkdown} />
                  </div>
                  <p className="mt-3 text-xs leading-relaxed text-muted-foreground">
                    Aucun aperçu natif pour ce type de fichier. Le markdown extrait ci-dessus est ce que le pipeline a masqué.
                  </p>
                </div>
              )
            ) : activeTab === "layout" ? (
              <div className="p-8">
                <div className="mx-auto max-w-lg rounded-lg border border-dashed border-border bg-card/50 p-8 text-center">
                  <LayoutGrid className="mx-auto size-6 text-muted-foreground" />
                  <p className="mt-3 text-sm font-medium">Aucune carte de mise en page pour ce document</p>
                  <p className="mt-1 text-xs leading-relaxed text-muted-foreground">
                    Les blocs de mise en page et les positions des encadrés sont produits pour les PDF scannés et les documents image. Ce document a été traité comme du texte brut — rien à afficher ici.
                  </p>
                </div>
              </div>
            ) : activeTab === "findings" ? (
              <div className="p-5">
                <div className="mb-3 flex items-center gap-2 text-[11px] font-medium uppercase tracking-widest text-muted-foreground">
                  <ScanSearch className="size-3.5" />
                  Document annoté
                  <span className="ml-auto inline-flex items-center gap-1 rounded-full border border-amber-500/20 bg-amber-500/10 px-2 py-0.5 font-mono text-[11px] normal-case tracking-normal text-amber-300">
                    <span className="size-1.5 rounded-full bg-amber-400" />
                    {findings.length} portion{findings.length === 1 ? "" : "s"} PII
                  </span>
                </div>
                <div className="rounded-lg border border-border bg-card p-5">
                  <PiiAnnotatedView text={result.rawMarkdown} findings={findings} />
                </div>
                <p className="mt-3 text-xs text-muted-foreground">
                  Cliquez sur une portion surlignée pour révéler sa catégorie. Gérez les faux positifs dans le panneau Détections →
                </p>
              </div>
            ) : activeTab === "audit" ? (
              <div className="p-5">
                <div className="mb-3 flex items-center gap-2 text-[11px] font-medium uppercase tracking-widest text-muted-foreground">
                  <ClipboardCheck className="size-3.5" />
                  Aperçu masqué
                  <span className="ml-auto rounded bg-muted px-1.5 py-0.5 font-mono text-[10px] normal-case tracking-normal text-muted-foreground">
                    {isPseudonymizeActive ? "pseudonymisé" : `mode ${mode}`}
                  </span>
                </div>
                <div className="rounded-lg border border-border bg-card p-5">
                  <PiiAnnotatedView text={result.rawMarkdown} findings={findings} />
                </div>
              </div>
            ) : (
              // redacted — default
              <div className="flex flex-col gap-5 p-5">
                <div>
                  <p className="mb-2 flex items-center gap-1.5 text-[11px] font-medium uppercase tracking-widest text-muted-foreground">
                    <ScanSearch className="size-3.5" />
                    Marquer une PII supplémentaire
                    <span className="ml-auto text-[11px] font-normal normal-case tracking-normal text-muted-foreground/70">
                      Sélectionnez une portion, puis étiquetez-la
                    </span>
                  </p>
                  <InteractiveEditor value={result.rawMarkdown} findings={findings} onAddFinding={onAddFinding} onRemoveFinding={onRemoveFinding} />
                </div>

                <div>
                  <p className="mb-2 flex items-center gap-1.5 text-[11px] font-medium uppercase tracking-widest text-muted-foreground">
                    <FileText className="size-3.5" />
                    Sortie masquée
                    <span className="ml-2 rounded bg-muted px-1.5 py-0.5 font-mono text-[10px] normal-case tracking-normal text-muted-foreground">
                      mode {mode}
                    </span>
                  </p>
                  {redactedBody === undefined ? (
                    <div className="rounded-lg border border-destructive/30 bg-destructive/10 p-4 text-sm text-destructive">
                      {"reason" in modeResult ? modeResult.reason : ""}
                    </div>
                  ) : originalFile ? (
                    <RedactedEditor value={redactedBody} onChange={setDraft} />
                  ) : (
                    <div className="rounded-lg border border-border bg-card">
                      <CodeLines text={redactedBody} />
                    </div>
                  )}
                </div>

                <p className="border-t border-border pt-3 text-xs leading-relaxed text-muted-foreground">
                  <span className="font-medium text-foreground">Astuce :</span> Basculez les modes de rédaction ci-dessus —{" "}
                  <span className="font-mono text-[11px]">Masquer</span> remplace par [CATÉGORIE],{" "}
                  <span className="font-mono text-[11px]">Hacher</span> par une empreinte,{" "}
                  <span className="font-mono text-[11px]">Pseudonymiser</span> par des jetons réversibles,{" "}
                  <span className="font-mono text-[11px]">Supprimer</span> supprime la portion.
                </p>
              </div>
            )}
          </div>

          {/* selection hint bar */}
          <div className="border-t border-border bg-card px-4 py-2 text-center text-[11px] text-muted-foreground">
            Sélectionnez du texte dans le document pour le marquer comme PII.
          </div>
        </div>

        {/* RIGHT PANEL — tab bar + details */}
        <div className="flex w-full shrink-0 flex-col bg-background lg:w-[400px] lg:max-w-[44vw]">
          {/* Tab pill — reuses the Switch's primary-tinted active/inactive language: the
           * active segment sits on `bg-card` with a subtle ring, same visual weight as the
           * Switch's checked thumb, for consistency across the redesign's toggle controls. */}
          <div className="border-b border-border bg-card px-3 py-2.5">
            <div
              role="tablist"
              aria-label="Sections du document"
              className="inline-flex w-full items-center gap-0.5 rounded-lg border border-border bg-muted/40 p-1"
            >
              {(["redacted", "source", "findings", "layout", "audit"] as const).map((tab) => {
                const active = activeTab === tab;
                return (
                  <button
                    key={tab}
                    role="tab"
                    aria-selected={active}
                    onClick={() => setActiveTab(tab)}
                    className={
                      active
                        ? "flex-1 rounded-md bg-card px-2.5 py-1.5 text-xs font-medium text-foreground shadow-sm ring-1 ring-primary/30"
                        : "flex-1 rounded-md px-2.5 py-1.5 text-xs font-medium text-muted-foreground hover:bg-accent hover:text-foreground"
                    }
                  >
                    {tab === "redacted"
                      ? "Masqué"
                      : tab === "source"
                        ? "Source"
                        : tab === "findings"
                          ? "Détections"
                          : tab === "layout"
                            ? "Mise en page"
                            : "Audit"}
                  </button>
                );
              })}
            </div>
          </div>

          {/* Right content */}
          <div className="flex-1 overflow-auto bg-background">
            {activeTab === "audit" ? (
              <AuditTab />
            ) : activeTab === "findings" ? (
              <div className="p-3">
                <div className="rounded-lg border border-border bg-card p-3">
                  <PiiPanel findings={findings} onRemove={onRemoveFinding} />
                </div>
                {findings.length > 0 && (
                  <div className="mt-3 rounded-lg border border-amber-500/15 bg-amber-500/[0.04] p-3">
                    <p className="text-xs font-medium text-amber-200">À propos de ces détections</p>
                    <p className="mt-1 text-xs leading-relaxed text-muted-foreground">
                      Les catégories proviennent du détecteur sur appareil. Les portions de faible confiance sont signalées — vérifiez avant d'exporter.
                    </p>
                  </div>
                )}
              </div>
            ) : activeTab === "source" ? (
              <div className="space-y-3 p-3">
                <div className="rounded-lg border border-border bg-card p-4">
                  <p className="text-xs font-medium uppercase tracking-widest text-muted-foreground">Fichier source</p>
                  <p className="mt-1 break-all font-mono text-xs text-foreground">{result.frontmatter.source}</p>
                  <div className="mt-3 grid grid-cols-2 gap-2 text-xs">
                    <div className="rounded-md bg-muted/40 px-2.5 py-2">
                      <p className="text-[11px] uppercase tracking-widest text-muted-foreground">Visualiseur</p>
                      <p className="mt-0.5 font-mono text-foreground">{viewerKind ?? "aucun"}</p>
                    </div>
                    <div className="rounded-md bg-muted/40 px-2.5 py-2">
                      <p className="text-[11px] uppercase tracking-widest text-muted-foreground">Octets</p>
                      <p className="mt-0.5 font-mono text-foreground">{originalFile ? `${originalFile.size.toLocaleString()} o` : "—"}</p>
                    </div>
                  </div>
                  {!hasViewer && (
                    <p className="mt-3 text-xs leading-relaxed text-muted-foreground">
                      L'aperçu natif prend en charge <span className="font-mono text-[11px]">pdf</span>,{" "}
                      <span className="font-mono text-[11px]">docx</span>,{" "}
                      <span className="font-mono text-[11px]">xlsx</span>,{" "}
                      <span className="font-mono text-[11px]">pptx</span> (et doc/xls/ppt hérités). Les autres formats s'affichent sous forme de markdown extrait.
                    </p>
                  )}
                </div>
                {hasViewer && (
                  <p className="px-1 text-xs text-muted-foreground">
                    L'aperçu natif défile dans le panneau gauche. Utilisez la barre d'outils du visualiseur pour zoomer, rechercher et paginer.
                  </p>
                )}
              </div>
            ) : activeTab === "layout" ? (
              <div className="p-3">
                <div className="rounded-lg border border-border bg-card p-4">
                  <p className="text-sm font-medium">Mise en page</p>
                  <p className="mt-1 text-xs leading-relaxed text-muted-foreground">
                    Les blocs et positions des encadrés sont produits pour les PDF scannés et les images. Ce document n'a pas de carte de mise en page — le markdown extrait dans le panneau gauche est l'extraction complète.
                  </p>
                </div>
              </div>
            ) : (
              // redacted — right summary
              <div className="space-y-3 p-3">
                <div className="rounded-lg border border-border bg-card p-4">
                  <p className="text-xs font-medium uppercase tracking-widest text-muted-foreground">Détections</p>
                  <p className="mt-1 flex items-baseline gap-2">
                    <span className="text-2xl font-semibold tabular-nums">{findings.length}</span>
                    <span className="text-xs text-muted-foreground">portion{findings.length === 1 ? "" : "s"} détectée{findings.length === 1 ? "" : "s"}</span>
                  </p>
                  {findings.length > 0 ? (
                    <ul className="mt-3 space-y-1.5">
                      {findings.slice(0, 6).map((f, i) => (
                        <li key={i} className="flex items-center justify-between gap-2 rounded-md bg-muted/30 px-2.5 py-1.5">
                          <span className="truncate font-mono text-xs">{f.redact_template || `[${f.category}]`}</span>
                          <span className="shrink-0 rounded bg-amber-500/15 px-1.5 py-0.5 text-[10px] font-bold uppercase tracking-wide text-amber-300">
                            {f.category}
                          </span>
                        </li>
                      ))}
                      {findings.length > 6 && (
                        <li className="px-2.5 py-1 text-xs text-muted-foreground">
                          +{findings.length - 6} de plus — ouvrez l'onglet Détections pour tout voir
                        </li>
                      )}
                    </ul>
                  ) : (
                    <p className="mt-2 text-xs text-muted-foreground">Aucune PII détectée. Utilisez l'éditeur pour marquer les portions manuellement.</p>
                  )}
                </div>
                <div className="rounded-lg border border-border bg-card p-4">
                  <p className="text-xs font-medium">Les exports sont déterministes</p>
                  <p className="mt-1 text-xs leading-relaxed text-muted-foreground">
                    <span className="font-mono text-[11px]">Masquer</span> et <span className="font-mono text-[11px]">Hacher</span> se redérivent instantanément. La pseudonymisation nécessite une clé définie au moment du traitement.
                  </p>
                </div>
              </div>
            )}
          </div>
        </div>
      </div>
    </div>
  );
}

/** Shown while a docx/xlsx/pptx viewer's lazy chunk loads. */
function ViewerLoadingFallback() {
  return (
    <div className="flex h-full min-h-[400px] items-center justify-center text-sm text-muted-foreground">
      Chargement du visualiseur…
    </div>
  );
}

function SourceReAddPrompt({
  viewerKind,
  fileName,
  onFilePicked,
  fallbackMarkdown,
}: {
  viewerKind: string;
  fileName: string;
  onFilePicked: (file: File) => void;
  fallbackMarkdown: string;
}) {
  const inputRef = useRef<HTMLInputElement>(null);
  const ext = fileName.split(".").pop()?.toLowerCase() ?? viewerKind;
  const accept =
    viewerKind === "pdf"
      ? ".pdf,application/pdf"
      : viewerKind === "docx" || viewerKind === "doc"
        ? ".docx,.doc,application/vnd.openxmlformats-officedocument.wordprocessingml.document"
        : viewerKind === "pptx" || viewerKind === "ppt"
          ? ".pptx,.ppt,application/vnd.openxmlformats-officedocument.presentationml.presentation"
          : ".xlsx,.xls,.xslt,.xlst,.xlsm,.xlsb,application/vnd.openxmlformats-officedocument.spreadsheetml.sheet";
  return (
    <div className="flex h-full min-h-[420px] flex-col items-center justify-center p-8 text-center">
      <input
        ref={inputRef}
        type="file"
        accept={accept}
        className="hidden"
        onChange={(e) => {
          const f = e.target.files?.[0];
          if (f) onFilePicked(f);
          e.target.value = "";
        }}
      />
      <div className="w-full max-w-md rounded-xl border border-dashed border-amber-500/25 bg-amber-500/[0.04] p-6">
        <div className="mx-auto flex size-9 items-center justify-center rounded-lg border border-border bg-card">
          <FileText className="size-4 text-muted-foreground" />
        </div>
        <p className="mt-3 text-sm font-medium">Aperçu natif {viewerKind.toUpperCase()} indisponible</p>
        <p className="mt-1 text-xs leading-relaxed text-muted-foreground">
          L'original <span className="font-mono text-[11px]">{fileName}</span> n'est conservé qu'en mémoire pour la session qui l'a traité. Ré-ajoutez-le pour afficher le visualiseur source pour{" "}
          <span className="font-mono text-[11px]">pdf</span>, <span className="font-mono text-[11px]">docx</span>,{" "}
          <span className="font-mono text-[11px]">pptx</span>, <span className="font-mono text-[11px]">xlsx</span> (et doc/xls/ppt hérités) — xslt correspond au visualiseur de feuille.
        </p>
        <Button size="sm" className="mt-4" onClick={() => inputRef.current?.click()}>
          Ré-ajouter le fichier {ext}
        </Button>
        <p className="mt-2 text-[11px] text-muted-foreground">Ou déposez le fichier n'importe où — le traitement reste sur l'appareil.</p>
      </div>
      <div className="mt-6 w-full max-w-md text-left">
        <p className="text-[11px] font-medium uppercase tracking-widest text-muted-foreground">Fallback — extracted markdown</p>
        <div className="mt-2 max-h-48 overflow-auto rounded-lg border border-border bg-card">
          <CodeLines text={fallbackMarkdown} />
        </div>
      </div>
    </div>
  );
}

/** Inline annotated view — original markdown with PII spans rendered as labelled chips. */
function PiiAnnotatedView({ text, findings }: { text: string; findings: PiiEntity[] }) {
  if (findings.length === 0) {
    return <pre className="whitespace-pre-wrap break-words font-mono text-xs leading-relaxed text-foreground/90">{text}</pre>;
  }
  const sorted = [...findings]
    .filter((f) => f.start >= 0 && f.end <= text.length && f.start < f.end)
    .sort((a, b) => a.start - b.start);

  // de-overlap: keep first span when overlapping, like decorations.ts
  const deduped: PiiEntity[] = [];
  let lastEnd = -1;
  for (const f of sorted) {
    if (f.start < lastEnd) continue;
    deduped.push(f);
    lastEnd = f.end;
  }

  const parts: React.ReactNode[] = [];
  let cursor = 0;
  for (let i = 0; i < deduped.length; i++) {
    const f = deduped[i];
    if (cursor < f.start) {
      parts.push(
        <span key={`t-${i}`} className="whitespace-pre-wrap break-words">
          {text.slice(cursor, f.start)}
        </span>,
      );
    }
    const raw = text.slice(f.start, f.end);
    parts.push(
      <span
        key={`p-${i}`}
        className="inline-flex max-w-full items-stretch overflow-hidden rounded border border-amber-500/25 align-baseline"
        style={{ verticalAlign: "baseline" }}
      >
        <span className="inline-flex items-center bg-amber-500/15 px-1.5 py-0.5 text-[10px] font-bold uppercase leading-none tracking-wide text-amber-300">
          {f.category}
        </span>
        <span className="inline-flex items-center bg-amber-950/40 px-1.5 py-0.5 font-mono text-xs leading-none text-amber-100/90">
          {raw}
        </span>
      </span>,
    );
    cursor = f.end;
  }
  if (cursor < text.length) {
    parts.push(
      <span key="t-end" className="whitespace-pre-wrap break-words">
        {text.slice(cursor)}
      </span>,
    );
  }

  return (
    <div className="whitespace-pre-wrap break-words font-mono text-xs leading-7 text-foreground/90">
      {parts}
    </div>
  );
}

/** `AuditEntryRow.action` renders as its variant name, or `"custom:<template>"` for the
 * one non-unit variant — matching `RedactionAction`'s own `Display` impl on the Rust side
 * (`hacienda-core/src/audit/entry.rs`) rather than inventing a second label scheme. */
function actionLabel(action: AuditEntryRow["action"]): string {
  return typeof action === "string" ? action : `custom:${action.custom}`;
}

function AuditTab() {
  const [tip, setTip] = useState<string | null>(null);
  const [entries, setEntries] = useState<AuditEntryRow[] | null>(null);
  const [status, setStatus] = useState<"idle" | "checking" | "ok" | "error">("idle");
  const [error, setError] = useState<string | null>(null);

  async function load() {
    try {
      const [chainTip, chainEntries] = await Promise.all([
        getAuditChainTip(),
        listAuditEntries(),
      ]);
      setTip(chainTip);
      // Newest first — matches the order a reader scanning "what just happened" expects.
      setEntries([...chainEntries].reverse());
    } catch (e) {
      setError(e instanceof Error ? e.message : "Chaîne d'audit indisponible");
    }
  }

  async function verify() {
    setStatus("checking");
    setError(null);
    try {
      await verifyAuditChain();
      setStatus("ok");
      toast.success("Chaîne vérifiée — aucune falsification détectée");
    } catch (e) {
      setStatus("error");
      const message = e instanceof Error ? e.message : "Échec de la vérification de la chaîne";
      setError(message);
      toast.error(message);
    }
  }

  useEffect(() => {
    load();
  }, []);

  return (
    <div className="flex flex-col">
      <div className="flex items-center gap-2 border-b border-border bg-card px-3 py-2.5">
        <ShieldCheck className="size-3.5 text-muted-foreground" />
        <span className="text-xs font-medium">Vérifier la chaîne</span>
        <button
          type="button"
          onClick={verify}
          disabled={status === "checking"}
          className="ml-auto rounded-md bg-foreground px-2.5 py-1 text-xs font-medium text-background hover:bg-foreground/90 disabled:opacity-50"
        >
          {status === "checking" ? "Vérification…" : "Vérifier"}
        </button>
      </div>

      <div className="space-y-0 divide-y divide-border">
        {entries === null ? (
          <div className="px-3 py-3">
            <p className="text-xs text-muted-foreground">
              {error ? "Chaîne d'audit indisponible." : "Chargement…"}
            </p>
          </div>
        ) : entries.length === 0 ? (
          <div className="px-3 py-3">
            <p className="text-xs text-muted-foreground">
              Aucune entrée enregistrée sur cet appareil pour l'instant.
            </p>
          </div>
        ) : (
          entries.map((entry) => (
            <div key={entry.id} className="px-3 py-3">
              <p className="font-mono text-[11px] leading-relaxed">
                <span className="font-medium text-amber-400">
                  {actionLabel(entry.action)}.{entry.category}
                </span>{" "}
                <span className="text-muted-foreground">
                  {new Date(entry.timestamp).toLocaleTimeString()}
                </span>
              </p>
              <p className="mt-0.5 break-all font-mono text-[11px] text-muted-foreground/70">
                {entry.chain_hash}
              </p>
            </div>
          ))
        )}

        {status === "ok" && (
          <p className="px-3 py-2 text-xs font-medium text-emerald-400">
            Chaîne vérifiée — aucune falsification détectée.
          </p>
        )}
        {status === "error" && error && (
          <p className="px-3 py-2 text-xs text-destructive">{error}</p>
        )}

        <div className="px-3 py-4">
          <div className="rounded-lg border border-border bg-card p-3">
            <p className="text-xs font-medium">Tête de chaîne</p>
            <p className="mt-1 break-all font-mono text-[11px] text-muted-foreground">
              {tip ?? (error ? "indisponible" : "chargement…")}
            </p>
          </div>
        </div>
      </div>
    </div>
  );
}
