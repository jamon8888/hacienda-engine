import { useEffect, useMemo, useRef, useState } from "react";
import { ArrowLeft, Download } from "lucide-react";
import { toast } from "sonner";
import { Button } from "@/components/ui/button";
import { Tabs, TabsList, TabsTrigger, TabsContent } from "@/components/ui/tabs";
import { CodeLines } from "@/components/CodeLines";
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

export function DocumentDetail({
  result,
  findings,
  originalFile,
  onBack,
  onAddFinding,
  onRemoveFinding,
  onExportBody,
}: {
  result: ProcessedFile;
  findings: PiiEntity[];
  /** The in-memory `File` this result was produced from, when this session's the one
   * that processed it — undefined for a document hydrated from a prior session's
   * persisted library (see `lib/persistence.ts`), where only the processed output
   * survives, never the original bytes. Drives both the Source tab's native viewer and
   * the Redacted tab's free-text draft autosave, since both need real file bytes. */
  originalFile: File | undefined;
  onBack: () => void;
  onAddFinding: (start: number, end: number, category: string) => void;
  onRemoveFinding: (index: number) => void;
  /** Exports the full document (frontmatter + entity glossary preserved) with `body` as
   * its redacted markdown — `App.tsx` owns the frontmatter/glossary wrap since it's the
   * only place with `result.markdown`'s split already worked out (see `reExportMarkdown`
   * there). */
  onExportBody: (body: string) => void;
}) {
  const [mode, setMode] = useState<RedactionMode>("mask");
  const [previewUrl, setPreviewUrl] = useState<string | undefined>(undefined);
  const [contentHash, setContentHash] = useState<string | undefined>(undefined);
  const [draft, setDraft] = useState<string | undefined>(undefined);

  const modeResult = useMemo(() => applyRedactionMode(findings, mode), [findings, mode]);
  const docPath = `documents/${result.name}`;
  const computedMarkdown =
    "findings" in modeResult
      ? renderAnnotatedMarkdown(result.rawMarkdown, result.entities, modeResult.findings, docPath)
      : null;

  const viewerKind = getViewerKind(result.frontmatter.source);
  const hasViewer = !!(viewerKind && previewUrl);

  // Object URL for the Source tab's native viewer, built from this-session bytes only —
  // revoked on cleanup/file change like `App.tsx`'s previous `previewUrls` map did per-entry.
  useEffect(() => {
    if (!originalFile) {
      setPreviewUrl(undefined);
      return;
    }
    const url = URL.createObjectURL(originalFile);
    setPreviewUrl(url);
    return () => URL.revokeObjectURL(url);
  }, [originalFile]);

  // Content hash keys the autosave draft to the original bytes, not the filename — see
  // `lib/content-hash.ts`'s header for why. Resets `draft` on file change so a stale
  // draft from a previously-open document can't bleed into this one.
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

  // Restore-on-open: adopts a saved draft only if nothing has been typed yet (`draft` is
  // still unset) by the time the lookup resolves.
  useEffect(() => {
    if (!contentHash) return;
    let cancelled = false;
    loadDraft(contentHash).then((saved) => {
      if (!cancelled && saved !== undefined) {
        setDraft((prev) => prev ?? saved);
        if (saved !== undefined) toast("Restored your last redacted draft for this file");
      }
    });
    return () => {
      cancelled = true;
    };
  }, [contentHash]);

  // Debounced (~1s) autosave — a toast on each successful save is the visible proof this
  // edit actually persisted (IndexedDB, keyed by content hash) rather than just living in
  // component state, since a reload/navigation away is exactly when a user wants to know
  // whether their edit survived.
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

  // A redaction-mode button is a deliberate reset, not a merge: it replaces whatever the
  // user was editing with a fresh computation for the newly-chosen mode.
  function selectMode(next: RedactionMode) {
    setMode(next);
    const nextResult = applyRedactionMode(findings, next);
    if ("findings" in nextResult) {
      setDraft(renderAnnotatedMarkdown(result.rawMarkdown, result.entities, nextResult.findings, docPath));
    }
  }

  const redactedBody = draft ?? computedMarkdown ?? undefined;

  return (
    <div className="flex flex-1 flex-col">
      <div className="flex items-center justify-between border-b border-border px-4 py-2">
        <div className="flex items-center gap-3">
          <Button variant="ghost" size="icon-sm" onClick={onBack}>
            <ArrowLeft className="size-4" />
          </Button>
          <span className="font-medium">{result.name}</span>
          <span className="rounded-full border border-border px-2 py-0.5 text-xs text-muted-foreground">
            {findings.length} finding{findings.length === 1 ? "" : "s"}
          </span>
        </div>
        <div className="flex items-center gap-2">
          {REDACTION_MODES.map((m) => (
            <Button
              key={m.value}
              size="sm"
              variant={mode === m.value ? "default" : "outline"}
              onClick={() => selectMode(m.value)}
            >
              {m.label}
            </Button>
          ))}
          <Button
            size="sm"
            variant="secondary"
            disabled={redactedBody === undefined}
            onClick={() => redactedBody !== undefined && onExportBody(redactedBody)}
          >
            <Download className="size-4" /> Redacted
          </Button>
        </div>
      </div>

      {!originalFile && (
        <div className="border-b border-border bg-muted/40 px-4 py-2 text-sm text-muted-foreground">
          Original not in this session — processed output is loaded from local storage,
          but the source file is only kept in memory for the session that produced it.
          Re-add the file to see its native preview and edit the redacted draft.
        </div>
      )}

      <Tabs defaultValue="redacted" className="flex-1">
        <TabsList className="mx-4 mt-3 w-fit">
          <TabsTrigger value="redacted">Redacted</TabsTrigger>
          <TabsTrigger value="source">Source</TabsTrigger>
          <TabsTrigger value="findings">Findings</TabsTrigger>
          <TabsTrigger value="layout">Layout</TabsTrigger>
          <TabsTrigger value="audit">Audit</TabsTrigger>
        </TabsList>

        {/* Redacted is the one editing surface in this view — marking new PII and
            free-text redaction edits both live here, not in Source (which is a pure
            viewer, native or read-only). */}
        <TabsContent value="redacted" className="flex-1 px-4 pb-4">
          <div className="flex flex-col gap-5">
            <div>
              <p className="mb-2 text-xs font-medium text-muted-foreground">
                Mark additional PII
              </p>
              <MarkdownEditor value={result.rawMarkdown} findings={findings} onAddFinding={onAddFinding} />
            </div>
            <div>
              <p className="mb-2 text-xs font-medium text-muted-foreground">Redacted output</p>
              {redactedBody === undefined ? (
                <p className="text-sm text-muted-foreground">
                  {"reason" in modeResult ? modeResult.reason : ""}
                </p>
              ) : originalFile ? (
                <RedactedEditor value={redactedBody} onChange={setDraft} />
              ) : (
                <CodeLines text={redactedBody} />
              )}
            </div>
          </div>
        </TabsContent>

        <TabsContent value="source" className="flex-1 px-4 pb-4">
          {hasViewer ? (
            <div className="h-[70vh] overflow-auto rounded-lg border border-border p-2">
              {(viewerKind === "docx" || viewerKind === "doc") && (
                <DocxViewerPreview src={previewUrl} fileName={result.name} isDark showUpload={false} onIsDarkChange={() => {}} />
              )}
              {(viewerKind === "xlsx" || viewerKind === "xls") && (
                <XlsxViewerPreview src={previewUrl} fileName={result.name} isDark showUpload={false} onIsDarkChange={() => {}} />
              )}
              {(viewerKind === "pptx" || viewerKind === "ppt") && (
                <PptxViewerPreview src={previewUrl} fileName={result.name} showUpload={false} />
              )}
              {viewerKind === "pdf" && <PDFViewer src={previewUrl} fileName={result.name} showUpload={false} />}
            </div>
          ) : (
            <CodeLines text={result.rawMarkdown} />
          )}
        </TabsContent>

        <TabsContent value="findings" className="flex-1 px-4 pb-4">
          <PiiPanel findings={findings} onRemove={onRemoveFinding} />
        </TabsContent>

        <TabsContent value="layout" className="flex-1 px-4 pb-4">
          <p className="p-4 text-sm text-muted-foreground">
            Layout data (block/bounding-box positions) isn't produced by this pipeline
            output yet — nothing to show for this document.
          </p>
        </TabsContent>

        <TabsContent value="audit" className="flex-1 px-4 pb-4">
          <AuditTab />
        </TabsContent>
      </Tabs>
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
    // eslint-disable-next-line react-hooks/exhaustive-deps -- load once per mount
  }, []);

  return (
    <div className="flex flex-col gap-3">
      <div className="rounded-lg border border-border bg-card p-4">
        <p className="text-sm font-medium">Chain tip</p>
        <p className="mt-1 break-all font-mono text-xs text-muted-foreground">
          {tip ?? (error ? "unavailable" : "loading…")}
        </p>
      </div>
      <Button className="w-fit" size="sm" onClick={verify} disabled={status === "checking"}>
        {status === "checking" ? "Verifying…" : "Verify chain"}
      </Button>
      {status === "ok" && (
        <p className="text-sm text-emerald-500">Chain verified — no tampering detected.</p>
      )}
      {status === "error" && <p className="text-sm text-destructive">{error}</p>}
      {error && status === "idle" && (
        <p className="text-xs text-muted-foreground">{error}</p>
      )}
    </div>
  );
}
