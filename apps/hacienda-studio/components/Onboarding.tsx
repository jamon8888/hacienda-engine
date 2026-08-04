import type { OnboardingState } from "../lib/types";
import type { DownloadProgress } from "../lib/asset-loader";

type AssetKey = keyof OnboardingState["assets"];

function formatMB(bytes: number): string {
  return `${(bytes / (1024 * 1024)).toFixed(0)}MB`;
}

const ICONS: Record<AssetKey, string> = {
  xbergWasm: "⚙️",
  nerModel: "🧠",
  tessdata: "👁️",
};

const LABELS: Record<AssetKey, string> = {
  xbergWasm: "xberg WASM Engine",
  nerModel: "GLiNER2-Guardrails-PII",
  tessdata: "Tesseract OCR Data",
};

function overallPercent(assets: OnboardingState["assets"]): number {
  const values = Object.values(assets);
  return Math.round((values.filter(Boolean).length / values.length) * 100);
}

export function Onboarding({
  assets,
  nerModelDegraded = false,
  nerDownloadProgress = null,
  onComplete,
}: {
  assets: OnboardingState["assets"];
  nerModelDegraded?: boolean;
  /** Byte-level progress for the in-flight ~614MB model download, or `null` when not
   * currently downloading (cached, not yet started, or already settled). Without this the
   * "GLiNER2-Guardrails-PII" row (and the overall percentage) sits frozen at a single fixed
   * value for the whole multi-minute download, indistinguishable from actually being stuck. */
  nerDownloadProgress?: DownloadProgress | null;
  onComplete: () => void;
}) {
  // `assets.nerModel` is true even when the neural backend failed and the app fell back
  // to regex-only detection — see App.tsx's preloadAssets. `nerModelDegraded` is the
  // only place that failure is recorded, so the status text must check it explicitly
  // instead of trusting `ready`.
  const statusFor = (key: AssetKey, ready: boolean): string => {
    if (key === "nerModel" && nerModelDegraded) return "⚠️ Unavailable — using fallback";
    if (key === "nerModel" && !ready && nerDownloadProgress) {
      const { receivedBytes, totalBytes } = nerDownloadProgress;
      // The network transfer can finish well before `assets.nerModel` flips true — the
      // ~600MB result still has to be written into IndexedDB, which on a memory-constrained
      // machine can itself take a long time. Without this, the row (and the overall
      // percentage) looks frozen at "100%" or 33% with no visible difference from actually
      // being stuck.
      if (totalBytes && receivedBytes >= totalBytes) {
        return "⏳ Finalizing... (saving to local cache)";
      }
      if (totalBytes) {
        const pct = Math.round((receivedBytes / totalBytes) * 100);
        return `↓ Downloading... ${pct}% (${formatMB(receivedBytes)} / ${formatMB(totalBytes)})`;
      }
      return `↓ Downloading... ${formatMB(receivedBytes)}`;
    }
    return ready ? "✓ Cached" : "↓ Downloading...";
  };

  const percent = overallPercent(assets);
  const allReady = assets.xbergWasm && assets.nerModel && assets.tessdata;

  return (
    <div
      className="onboarding-overlay fixed inset-0 z-[1000] flex items-center justify-center bg-black/90 p-4"
      role="dialog"
      aria-modal="true"
      aria-labelledby="onboarding-title"
    >
      <div className="w-full max-w-[520px] rounded-xl border border-border bg-card p-8 text-card-foreground shadow-2xl">
        <header className="mb-6 text-center">
          <h1 id="onboarding-title" className="mb-2 text-2xl font-semibold">
            <span aria-hidden="true" className="mr-2">
              🔒
            </span>
            Hacienda Studio — 100% Local AI in Your Browser
          </h1>
          <p className="text-sm leading-relaxed text-muted-foreground">
            Your files <strong>NEVER leave this tab</strong>. All processing runs locally
            via WebAssembly.
          </p>
        </header>

        <div className="mb-4">
          <div
            className="mb-2 h-2 overflow-hidden rounded-full bg-muted"
            role="progressbar"
            aria-valuenow={percent}
            aria-valuemin={0}
            aria-valuemax={100}
          >
            <div
              className="h-full bg-primary transition-[width] duration-300 ease-out"
              style={{ width: `${percent}%` }}
            />
          </div>
          <span className="text-sm text-muted-foreground">
            {percent}% — Preparing local models...
          </span>
        </div>

        <ul className="mb-6 flex flex-col gap-2" role="list">
          {(Object.keys(assets) as AssetKey[]).map((key) => {
            const ready = assets[key];
            const degraded = key === "nerModel" && nerModelDegraded;
            return (
              <li
                key={key}
                className={
                  "flex items-center gap-2 rounded-md bg-muted/40 p-3 border-l-[3px] " +
                  (degraded || !ready ? "border-l-amber-500" : "border-l-emerald-500")
                }
              >
                <span aria-hidden="true" className="text-lg">
                  {ICONS[key]}
                </span>
                <span className="flex-1 font-medium">{LABELS[key]}</span>
                <span
                  className={
                    "text-sm " +
                    (degraded ? "text-amber-600" : ready ? "text-emerald-600" : "text-muted-foreground")
                  }
                >
                  {statusFor(key, ready)}
                </span>
              </li>
            );
          })}
        </ul>

        <footer className="text-center">
          <button
            className="rounded-md bg-secondary px-6 py-2 font-medium text-secondary-foreground transition-colors hover:bg-primary hover:text-primary-foreground disabled:cursor-not-allowed disabled:opacity-50"
            disabled={!allReady}
            onClick={onComplete}
          >
            Continue
          </button>
        </footer>
      </div>
    </div>
  );
}
