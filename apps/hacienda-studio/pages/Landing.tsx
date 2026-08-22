import { Lock, ScanEye, Layers, ShieldCheck, EyeOff, Server } from "lucide-react";
import { Button } from "@/components/ui/button";

const FEATURES = [
  {
    icon: Lock,
    title: "Extract anything",
    body: "PDF, DOCX, XLSX, PPTX, CSV and plain text are converted to clean markdown with a layout map.",
  },
  {
    icon: EyeOff,
    title: "Find personal data",
    body: "PII is surfaced with confidence scores, tuned by sensitivity and vertical presets.",
  },
  {
    icon: Layers,
    title: "Redact your way",
    body: "Mask, hash, pseudonymize or remove each span — previewed live against the original.",
  },
  {
    icon: ScanEye,
    title: "Review and export",
    body: "Edit the redacted markdown by hand, then export a zip with originals, redactions and findings.",
  },
  {
    icon: ShieldCheck,
    title: "Tamper-evident trail",
    body: "Every redaction appends to a hash-chained local audit log you can verify at any time.",
  },
  {
    icon: Server,
    title: "No server, no account",
    body: "Documents are processed in a Web Worker and cached in IndexedDB on your own device.",
  },
];

import type { OnboardingState } from "@/lib/types";

const ASSET_LABELS: Record<keyof OnboardingState["assets"], string> = {
  xbergWasm: "Pipeline runtime",
  nerModel: "Entity recognition model",
  tessdata: "Optical recognition data",
};

export function Landing({
  assets,
  nerModelProgress,
  nerModelDegraded,
  onPrepare,
  onSkip,
}: {
  assets: OnboardingState["assets"];
  nerModelProgress: { receivedBytes: number; totalBytes: number | null } | null;
  nerModelDegraded: boolean;
  onPrepare: () => void;
  onSkip: () => void;
}) {
  return (
    <div className="flex-1">
      <section className="border-b border-border px-6 py-24">
        <div className="mx-auto max-w-3xl">
          <span className="inline-flex items-center gap-2 rounded-full border border-border bg-card px-3 py-1 text-xs text-muted-foreground">
            <Lock className="size-3" /> Local-first pipeline · zero uploads
          </span>
          {/* Asset loading progress on first visit - single bar */}
          {(!assets.xbergWasm || !assets.nerModel || !assets.tessdata) && (() => {
            const allReady = assets.xbergWasm && assets.nerModel && assets.tessdata;
            let label = "Préparation du workspace";
            let pct = 0;
            if (!assets.xbergWasm) {
              label = "Chargement du runtime…";
              pct = 0;
            } else if (!assets.nerModel) {
              label = nerModelDegraded ? "Modèle neural indisponible — fallback regex" : "Téléchargement du modèle d’entités…";
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
          <h1 className="mt-6 text-5xl font-semibold leading-tight tracking-tight">
            Redact sensitive documents without letting them leave your laptop.
          </h1>
          <p className="mt-6 max-w-xl text-base leading-relaxed text-muted-foreground">
            Hacienda Studio runs the whole extraction, detection and redaction pipeline
            in your browser. Drop files in, review what the pipeline found, correct it,
            and export clean copies with an auditable trail.
          </p>
          <div className="mt-8 flex items-center gap-3">
            <Button size="lg" onClick={onPrepare}>
              Prepare the workspace →
            </Button>
            <Button size="lg" variant="outline" onClick={onSkip}>
              Skip to the studio
            </Button>
          </div>
          <div className="mt-10 flex flex-wrap items-center gap-2 text-xs text-muted-foreground">
            {["Extract", "Recognise", "Chunk", "Entities", "PII", "Redact"].map(
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

      <section className="border-b border-border px-6 py-16">
        <div className="mx-auto max-w-5xl">
          <h2 className="mb-8 text-2xl font-semibold">What the studio does</h2>
          <div className="grid grid-cols-1 gap-px overflow-hidden rounded-lg border border-border bg-border sm:grid-cols-3">
            {FEATURES.map(({ icon: Icon, title, body }) => (
              <div key={title} className="bg-card p-6">
                <Icon className="mb-4 size-5 text-primary" />
                <h3 className="mb-1.5 font-semibold">{title}</h3>
                <p className="text-sm text-muted-foreground">{body}</p>
              </div>
            ))}
          </div>
        </div>
      </section>

      <section className="px-6 py-16">
        <div className="mx-auto flex max-w-5xl items-center justify-between gap-6">
          <div>
            <h2 className="text-2xl font-semibold">Ready when you are</h2>
            <p className="mt-2 text-muted-foreground">
              Load the on-device assets once; everything after that works offline.
            </p>
          </div>
          <Button size="lg" onClick={onPrepare}>
            Start setup
          </Button>
        </div>
      </section>

      <footer className="border-t border-border px-6 py-8 text-center text-sm text-muted-foreground">
        Hacienda Studio — documents are processed on-device. No accounts, no server storage.
      </footer>
    </div>
  );
}
