import { useEffect, useMemo, useState } from "react";
import { ArrowLeft, Download } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Tabs, TabsList, TabsTrigger, TabsContent } from "@/components/ui/tabs";
import { CodeLines } from "@/components/CodeLines";
import { MarkdownEditor } from "@/components/MarkdownEditor";
import { PiiPanel } from "@/components/PiiPanel";
import { renderAnnotatedMarkdown } from "@/lib/annotate";
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
  originalAvailable,
  onBack,
  onAddFinding,
  onRemoveFinding,
  onExport,
}: {
  result: ProcessedFile;
  findings: PiiEntity[];
  originalAvailable: boolean;
  onBack: () => void;
  onAddFinding: (start: number, end: number, category: string) => void;
  onRemoveFinding: (index: number) => void;
  /** Exports the full document (frontmatter + entity glossary preserved) redacted with
   * the given findings — `App.tsx` owns this since it needs `result.markdown`'s
   * frontmatter/glossary split, which this view doesn't otherwise touch. */
  onExport: (findings: PiiEntity[]) => void;
}) {
  const [mode, setMode] = useState<RedactionMode>("mask");

  const modeResult = useMemo(() => applyRedactionMode(findings, mode), [findings, mode]);
  const docPath = `documents/${result.name}`;
  const redactedMarkdown =
    "findings" in modeResult
      ? renderAnnotatedMarkdown(result.rawMarkdown, result.entities, modeResult.findings, docPath)
      : null;

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
              onClick={() => setMode(m.value)}
            >
              {m.label}
            </Button>
          ))}
          <Button
            size="sm"
            variant="secondary"
            disabled={!("findings" in modeResult)}
            onClick={() => "findings" in modeResult && onExport(modeResult.findings)}
          >
            <Download className="size-4" /> Redacted
          </Button>
        </div>
      </div>

      {!originalAvailable && (
        <div className="border-b border-border bg-muted/40 px-4 py-2 text-sm text-muted-foreground">
          Original not in this session — processed output is loaded from local storage,
          but the source file is only kept in memory for the session that produced it.
          Re-add the file to see its native preview.
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

        <TabsContent value="redacted" className="flex-1 px-4 pb-4">
          {redactedMarkdown !== null ? (
            <CodeLines text={redactedMarkdown} />
          ) : (
            <p className="p-4 text-sm text-muted-foreground">
              {"reason" in modeResult ? modeResult.reason : ""}
            </p>
          )}
        </TabsContent>

        <TabsContent value="source" className="flex-1 px-4 pb-4">
          <MarkdownEditor
            value={result.rawMarkdown}
            findings={findings}
            onAddFinding={onAddFinding}
          />
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
    } catch (e) {
      setStatus("error");
      setError(e instanceof Error ? e.message : "Chain verification failed");
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
