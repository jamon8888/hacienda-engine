/**
 * Page rendering of the same asset-prep state `components/Onboarding.tsx` (the old modal)
 * showed — same `OnboardingState["assets"]` shape, same `lib/asset-loader.ts` downloads
 * that `App.tsx`'s `preloadAssets` effect already kicks off on mount. Not a new download
 * flow: this page only changes how that in-flight state is presented (a full page instead
 * of a blocking overlay), so "Continue without loading" is a real affordance here — the
 * app already tolerates a degraded/regex-only PII backend (`nerModelDegraded`), it just
 * never exposed a way to move on before assets finished without leaving the modal stuck.
 */
import { Download } from "lucide-react";
import type { OnboardingState } from "@/lib/types";
import { Button } from "@/components/ui/button";

type AssetKey = keyof OnboardingState["assets"];

function formatMB(bytes: number): string {
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}

const ASSET_INFO: Record<
  AssetKey,
  { label: string; description: string; sizeMB: number; optional: boolean }
> = {
  xbergWasm: {
    label: "Runtime du pipeline",
    description: "Cœur d'analyse et de rédaction des documents",
    sizeMB: 48,
    optional: false,
  },
  nerModel: {
    label: "Modèle de reconnaissance d'entités",
    description: "Modèle d'entités nommées pour personnes, organisations et lieux",
    sizeMB: 1170,
    optional: true,
  },
  tessdata: {
    label: "Données de reconnaissance optique",
    description: "Données en anglais pour les pages scannées",
    sizeMB: 22,
    optional: true,
  },
};

export function Assets({
  assets,
  nerModelDegraded,
  nerModelProgress,
  onContinue,
}: {
  assets: OnboardingState["assets"];
  nerModelDegraded: boolean;
  nerModelProgress: { receivedBytes: number; totalBytes: number | null } | null;
  onContinue: () => void;
}) {
  const allReady = assets.xbergWasm && assets.nerModel && assets.tessdata;

  return (
    <div className="flex flex-1 flex-col px-6 py-16">
      <h1 className="text-2xl font-semibold">Préparer l'espace de travail</h1>
      <p className="mt-2 max-w-lg text-muted-foreground">
        Ces ressources sont mises en cache sur votre appareil pour un traitement hors ligne. Les ressources mises en cache sont automatiquement ignorées lors des visites futures.
      </p>

      <div className="mt-8 flex flex-col gap-3">
        {(Object.keys(assets) as AssetKey[]).map((key) => {
          const info = ASSET_INFO[key];
          const ready = assets[key];
          const degraded = key === "nerModel" && nerModelDegraded;
          const isDownloading = key === "nerModel" && !ready && !degraded && nerModelProgress;
          const progressPct = isDownloading && nerModelProgress?.totalBytes
            ? Math.min(100, Math.round((nerModelProgress.receivedBytes / nerModelProgress.totalBytes) * 100))
            : 0;
          const barWidth = ready ? "100%" : isDownloading && nerModelProgress?.totalBytes ? `${progressPct}%` : "0%";
          return (
            <div key={key} className="rounded-lg border border-border bg-card p-4">
              <div className="flex items-center gap-2">
                {ready ? (
                  <span className="flex size-5 items-center justify-center rounded-full bg-emerald-500/20 text-emerald-500">
                    ✓
                  </span>
                ) : (
                  <Download className="size-5 text-muted-foreground" />
                )}
                <span className="font-semibold">{info.label}</span>
                <span className="text-xs text-muted-foreground">
                  {formatMB(info.sizeMB * 1024 * 1024)}
                </span>
                {info.optional && (
                  <span className="rounded border border-border px-1.5 py-0.5 text-[10px] uppercase tracking-wide text-muted-foreground">
                    Optionnel
                  </span>
                )}
              </div>
              <p className="mt-1 text-sm text-muted-foreground">{info.description}</p>
              <div className="mt-3 h-1.5 overflow-hidden rounded-full bg-muted">
                <div
                  className="h-full bg-primary transition-[width] duration-300 ease-out"
                  style={{ width: barWidth }}
                />
              </div>
              <p className="mt-1 text-xs text-muted-foreground">
                {degraded
                  ? "Indisponible — retour à la détection regex uniquement"
                  : ready
                    ? "Prêt"
                    : isDownloading && nerModelProgress?.totalBytes
                      ? `Téléchargement… ${progressPct}%`
                      : "Téléchargement…"}
              </p>
            </div>
          );
        })}
      </div>

      <div className="mt-6 flex items-center gap-3">
        <Button size="lg" disabled={!allReady} onClick={onContinue}>
          {allReady ? "Ouvrir le studio" : "Chargement des ressources…"}
        </Button>
        {!allReady && (
          <Button size="lg" variant="outline" onClick={onContinue}>
            Continuer sans charger
          </Button>
        )}
      </div>
    </div>
  );
}
