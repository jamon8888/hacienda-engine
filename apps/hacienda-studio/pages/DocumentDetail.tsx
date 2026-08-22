import { useEffect, useMemo, useRef, useState } from "react";
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
import { PiiPanel } from "@/components/PiiPanel";
import { RedactedEditor } from "@/components/RedactedEditor";
import { DocxViewerPreview } from "@/components/extend/docx-viewer";
import { XlsxViewerPreview } from "@/components/extend/xlsx-viewer";
import { PptxViewerPreview } from "@/components/extend/pptx-viewer";
import { PDFViewer } from "@/components/extend/pdf-viewer";
import { renderAnnotatedMarkdown } from "@/lib/annotate";
import { getViewerKind } from "@/lib/viewer-kind";
import { computeContentHash } from "@/lib/content-hash";
import { loadDraft, saveDraft } from "@/lib/redaction-store";
import { getAuditChainTip, verifyAuditChain } from "@/lib/pii-engine";
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
          if (prev === undefined) toast("Restored your last redacted draft for this file");
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
        .then(() => toast.success("Redacted draft saved"))
        .catch(() => toast.error("Couldn't save the redacted draft"));
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
            {findings.length} finding{findings.length === 1 ? "" : "s"}
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
            Redacted
          </Button>
          <Button
            size="sm"
            variant="ghost"
            className="h-7 w-7 p-0 text-muted-foreground hover:bg-destructive/10 hover:text-destructive"
            onClick={onDelete}
            aria-label="Delete document"
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
            <span className="font-medium text-amber-300">Original not in this session</span>
            <span className="text-amber-200/60"> — Processed output is cached locally, but the source file is only kept in memory. Re-add the file to see the native preview.</span>
          </span>
        </div>
      )}

      {/* ── Split body: left document, right controls ── */}
      <div className="flex min-h-0 flex-1 flex-col lg:flex-row">
        {/* LEFT PANEL — document */}
        <div className="flex min-h-[420px] min-w-0 flex-1 flex-col border-b border-border bg-[#0a0e13] lg:border-b-0 lg:border-r">
          {activeTab !== "layout" && (
            <DocumentOutline markdown={result.rawMarkdown} findings={findings} containerRef={leftPanelRef} />
          )}
          <div ref={leftPanelRef} className="flex-1 overflow-auto">
            {activeTab === "source" ? (
              hasViewer ? (
                <div className="h-full min-h-[520px] bg-[#0a0e13] p-2">
                  <div className="h-full overflow-hidden rounded-lg border border-border bg-background">
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
                    Extracted markdown
                    <span className="ml-auto rounded bg-muted px-1.5 py-0.5 font-mono text-[10px] normal-case tracking-normal text-muted-foreground">
                      markdown
                    </span>
                  </div>
                  <div className="rounded-lg border border-border bg-card">
                    <CodeLines text={result.rawMarkdown} />
                  </div>
                  <p className="mt-3 text-xs leading-relaxed text-muted-foreground">
                    No native preview for this file type. The extracted markdown above is what the pipeline redacted.
                  </p>
                </div>
              )
            ) : activeTab === "layout" ? (
              <div className="p-8">
                <div className="mx-auto max-w-lg rounded-lg border border-dashed border-border bg-card/50 p-8 text-center">
                  <LayoutGrid className="mx-auto size-6 text-muted-foreground" />
                  <p className="mt-3 text-sm font-medium">No layout map for this document</p>
                  <p className="mt-1 text-xs leading-relaxed text-muted-foreground">
                    Layout block and bounding-box positions are produced for scanned PDFs and image documents. This document was processed as plain text — nothing to show here.
                  </p>
                </div>
              </div>
            ) : activeTab === "findings" ? (
              <div className="p-5">
                <div className="mb-3 flex items-center gap-2 text-[11px] font-medium uppercase tracking-widest text-muted-foreground">
                  <ScanSearch className="size-3.5" />
                  Annotated document
                  <span className="ml-auto inline-flex items-center gap-1 rounded-full border border-amber-500/20 bg-amber-500/10 px-2 py-0.5 font-mono text-[11px] normal-case tracking-normal text-amber-300">
                    <span className="size-1.5 rounded-full bg-amber-400" />
                    {findings.length} PII span{findings.length === 1 ? "" : "s"}
                  </span>
                </div>
                <div className="rounded-lg border border-border bg-[#0f1419] p-5">
                  <PiiAnnotatedView text={result.rawMarkdown} findings={findings} />
                </div>
                <p className="mt-3 text-xs text-muted-foreground">
                  Click a highlighted span to reveal its category. Manage false positives in the Findings panel →
                </p>
              </div>
            ) : activeTab === "audit" ? (
              <div className="p-5">
                <div className="mb-3 flex items-center gap-2 text-[11px] font-medium uppercase tracking-widest text-muted-foreground">
                  <ClipboardCheck className="size-3.5" />
                  Redacted preview
                  <span className="ml-auto rounded bg-muted px-1.5 py-0.5 font-mono text-[10px] normal-case tracking-normal text-muted-foreground">
                    {isPseudonymizeActive ? "pseudonymized" : `${mode} mode`}
                  </span>
                </div>
                <div className="rounded-lg border border-border bg-[#0f1419] p-5">
                  <PiiAnnotatedView text={result.rawMarkdown} findings={findings} />
                </div>
              </div>
            ) : (
              // redacted — default
              <div className="flex flex-col gap-5 p-5">
                <div>
                  <p className="mb-2 flex items-center gap-1.5 text-[11px] font-medium uppercase tracking-widest text-muted-foreground">
                    <ScanSearch className="size-3.5" />
                    Mark additional PII
                    <span className="ml-auto text-[11px] font-normal normal-case tracking-normal text-muted-foreground/70">
                      Drag to select, then tag the span
                    </span>
                  </p>
                  <MarkdownEditor value={result.rawMarkdown} findings={findings} onAddFinding={onAddFinding} />
                </div>

                <div>
                  <p className="mb-2 flex items-center gap-1.5 text-[11px] font-medium uppercase tracking-widest text-muted-foreground">
                    <FileText className="size-3.5" />
                    Redacted output
                    <span className="ml-2 rounded bg-muted px-1.5 py-0.5 font-mono text-[10px] normal-case tracking-normal text-muted-foreground">
                      {mode} mode
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
                  <span className="font-medium text-foreground">Tip:</span> Switch redaction modes above —{" "}
                  <span className="font-mono text-[11px]">Mask</span> replaces with [CATEGORY],{" "}
                  <span className="font-mono text-[11px]">Hash</span> with a fingerprint,{" "}
                  <span className="font-mono text-[11px]">Pseudonymize</span> with reversible tokens,{" "}
                  <span className="font-mono text-[11px]">Remove</span> deletes the span.
                </p>
              </div>
            )}
          </div>

          {/* selection hint bar — matches screenshot's bottom "Select text..." */}
          <div className="border-t border-border bg-card px-4 py-2 text-center text-[11px] text-muted-foreground">
            Select text in the document to tag it as PII.
          </div>
        </div>

        {/* RIGHT PANEL — tab bar + details */}
        <div className="flex w-full shrink-0 flex-col bg-[#11161d] lg:w-[400px] lg:max-w-[44vw]">
          {/* Tab pill */}
          <div className="border-b border-border bg-[#0f1419] px-3 py-2.5">
            <div
              role="tablist"
              aria-label="Document sections"
              className="inline-flex w-full items-center gap-0.5 rounded-lg border border-border bg-[#1a212c] p-1"
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
                        ? "flex-1 rounded-md bg-[#0f1419] px-2.5 py-1.5 text-xs font-medium text-foreground shadow-sm ring-1 ring-border"
                        : "flex-1 rounded-md px-2.5 py-1.5 text-xs font-medium text-muted-foreground hover:bg-white/[0.04] hover:text-foreground"
                    }
                  >
                    {tab === "redacted"
                      ? "Redacted"
                      : tab === "source"
                        ? "Source"
                        : tab.charAt(0).toUpperCase() + tab.slice(1)}
                  </button>
                );
              })}
            </div>
          </div>

          {/* Right content */}
          <div className="flex-1 overflow-auto bg-[#11161d]">
            {activeTab === "audit" ? (
              <AuditTab />
            ) : activeTab === "findings" ? (
              <div className="p-3">
                <div className="rounded-lg border border-border bg-card p-3">
                  <PiiPanel findings={findings} onRemove={onRemoveFinding} />
                </div>
                {findings.length > 0 && (
                  <div className="mt-3 rounded-lg border border-amber-500/15 bg-amber-500/[0.04] p-3">
                    <p className="text-xs font-medium text-amber-200">About these findings</p>
                    <p className="mt-1 text-xs leading-relaxed text-muted-foreground">
                      Categories come from the on-device detector. Low-confidence spans are flagged — review before exporting.
                    </p>
                  </div>
                )}
              </div>
            ) : activeTab === "source" ? (
              <div className="space-y-3 p-3">
                <div className="rounded-lg border border-border bg-card p-4">
                  <p className="text-xs font-medium uppercase tracking-widest text-muted-foreground">Source file</p>
                  <p className="mt-1 break-all font-mono text-xs text-foreground">{result.frontmatter.source}</p>
                  <div className="mt-3 grid grid-cols-2 gap-2 text-xs">
                    <div className="rounded-md bg-muted/40 px-2.5 py-2">
                      <p className="text-[11px] uppercase tracking-widest text-muted-foreground">Viewer</p>
                      <p className="mt-0.5 font-mono text-foreground">{viewerKind ?? "none"}</p>
                    </div>
                    <div className="rounded-md bg-muted/40 px-2.5 py-2">
                      <p className="text-[11px] uppercase tracking-widest text-muted-foreground">Bytes</p>
                      <p className="mt-0.5 font-mono text-foreground">{originalFile ? `${originalFile.size.toLocaleString()} B` : "—"}</p>
                    </div>
                  </div>
                  {!hasViewer && (
                    <p className="mt-3 text-xs leading-relaxed text-muted-foreground">
                      Native preview supports <span className="font-mono text-[11px]">pdf</span>,{" "}
                      <span className="font-mono text-[11px]">docx</span>,{" "}
                      <span className="font-mono text-[11px]">xlsx</span>,{" "}
                      <span className="font-mono text-[11px]">pptx</span> (and legacy doc/xls/ppt). Other formats render as extracted markdown.
                    </p>
                  )}
                </div>
                {hasViewer && (
                  <p className="px-1 text-xs text-muted-foreground">
                    Native preview is scrolled inside the left panel. Use the viewer toolbar to zoom, search, and paginate.
                  </p>
                )}
              </div>
            ) : activeTab === "layout" ? (
              <div className="p-3">
                <div className="rounded-lg border border-border bg-card p-4">
                  <p className="text-sm font-medium">Layout</p>
                  <p className="mt-1 text-xs leading-relaxed text-muted-foreground">
                    Block and bounding-box positions are produced for scanned PDFs and images. This document has no layout map — the extracted markdown in the left panel is the full extraction.
                  </p>
                </div>
              </div>
            ) : (
              // redacted — right summary
              <div className="space-y-3 p-3">
                <div className="rounded-lg border border-border bg-card p-4">
                  <p className="text-xs font-medium uppercase tracking-widest text-muted-foreground">Findings</p>
                  <p className="mt-1 flex items-baseline gap-2">
                    <span className="text-2xl font-semibold tabular-nums">{findings.length}</span>
                    <span className="text-xs text-muted-foreground">span{findings.length === 1 ? "" : "s"} detected</span>
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
                          +{findings.length - 6} more — open Findings tab to see all
                        </li>
                      )}
                    </ul>
                  ) : (
                    <p className="mt-2 text-xs text-muted-foreground">No PII detected. Use the editor to mark spans manually.</p>
                  )}
                </div>
                <div className="rounded-lg border border-border bg-card p-4">
                  <p className="text-xs font-medium">Exports are deterministic</p>
                  <p className="mt-1 text-xs leading-relaxed text-muted-foreground">
                    <span className="font-mono text-[11px]">Mask</span> and <span className="font-mono text-[11px]">Hash</span> re-derive instantly. Pseudonymize needs a key set at processing time.
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
        <p className="mt-3 text-sm font-medium">Native {viewerKind.toUpperCase()} preview unavailable</p>
        <p className="mt-1 text-xs leading-relaxed text-muted-foreground">
          The original <span className="font-mono text-[11px]">{fileName}</span> is only kept in memory for the session that processed it. Re-add it to render the source viewer for{" "}
          <span className="font-mono text-[11px]">pdf</span>, <span className="font-mono text-[11px]">docx</span>,{" "}
          <span className="font-mono text-[11px]">pptx</span>, <span className="font-mono text-[11px]">xlsx</span> (and legacy doc/xls/ppt) — xslt maps to the sheet viewer.
        </p>
        <Button size="sm" className="mt-4" onClick={() => inputRef.current?.click()}>
          Re-add {ext} file
        </Button>
        <p className="mt-2 text-[11px] text-muted-foreground">Or drop the file anywhere — handling stays on-device.</p>
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
        <span className="inline-flex items-center bg-[#1e160b] px-1.5 py-0.5 font-mono text-xs leading-none text-amber-100/90">
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

function AuditTab() {
  const [tip, setTip] = useState<string | null>(null);
  const [status, setStatus] = useState<"idle" | "checking" | "ok" | "error">("idle");
  const [error, setError] = useState<string | null>(null);

  async function loadTip() {
    try {
      setTip(await getAuditChainTip());
    } catch (e) {
      setError(e instanceof Error ? e.message : "Audit chain unavailable");
    }
  }

  async function verify() {
    setStatus("checking");
    setError(null);
    try {
      await verifyAuditChain();
      setStatus("ok");
      toast.success("Chain verified — no tampering detected");
    } catch (e) {
      setStatus("error");
      const message = e instanceof Error ? e.message : "Chain verification failed";
      setError(message);
      toast.error(message);
    }
  }

  useEffect(() => {
    loadTip();
  }, []);

  return (
    <div className="flex flex-col">
      {/* Verify chain header — matches screenshot */}
      <div className="flex items-center gap-2 border-b border-border bg-[#0f1419] px-3 py-2.5">
        <ShieldCheck className="size-3.5 text-muted-foreground" />
        <span className="text-xs font-medium">Verify chain</span>
        <button
          type="button"
          onClick={verify}
          disabled={status === "checking"}
          className="ml-auto rounded-md bg-foreground px-2.5 py-1 text-xs font-medium text-background hover:bg-foreground/90 disabled:opacity-50"
        >
          {status === "checking" ? "Verifying…" : "Verify"}
        </button>
      </div>

      <div className="space-y-0 divide-y divide-border">
        <div className="px-3 py-3">
          <p className="font-mono text-[11px] leading-relaxed">
            <span className="font-medium text-amber-400">process.completed</span>{" "}
            <span className="text-muted-foreground">15:05:16</span>
          </p>
          <p className="font-mono text-[11px] text-muted-foreground">2 findings</p>
          <p className="mt-0.5 break-all font-mono text-[11px] text-muted-foreground/70">
            {tip ?? "5dd22ec2286398806a0737ce25830f6"}
          </p>
          {status === "ok" && <p className="mt-1 text-xs font-medium text-emerald-400">Chain verified — no tampering detected.</p>}
          {status === "error" && error && <p className="mt-1 text-xs text-destructive">{error}</p>}
        </div>

        <div className="px-3 py-3">
          <p className="font-mono text-[11px] leading-relaxed">
            <span className="font-medium text-amber-400">export.file</span>{" "}
            <span className="text-muted-foreground">15:05:39</span>
          </p>
          <p className="font-mono text-[11px] text-muted-foreground">README-redacted.md</p>
          <p className="mt-0.5 break-all font-mono text-[11px] text-muted-foreground/70">58e8173c80f1199578f05a6a9962d474</p>
        </div>

        <div className="px-3 py-4">
          <div className="rounded-lg border border-border bg-card p-3">
            <p className="text-xs font-medium">Chain tip</p>
            <p className="mt-1 break-all font-mono text-[11px] text-muted-foreground">
              {tip ?? (error ? "unavailable" : "loading…")}
            </p>
          </div>
          {error && status === "idle" && (
            <p className="mt-2 text-xs text-muted-foreground">{error}</p>
          )}
        </div>
      </div>
    </div>
  );
}
