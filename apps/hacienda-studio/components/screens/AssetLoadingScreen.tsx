/**
 * Screen 1 of 5: local-model preload. Thin wrapper around `Onboarding.tsx`, which already
 * has the right shape (per-asset progress rows, byte-level NER download progress) — kept as
 * its own file for directory-structure consistency with the other four screens, not because
 * there's new logic here.
 */
import { Onboarding } from "../Onboarding";
import type { OnboardingState } from "../../lib/types";
import type { DownloadProgress } from "../../lib/asset-loader";

export function AssetLoadingScreen({
  assets,
  nerModelDegraded,
  nerDownloadProgress,
  onComplete,
}: {
  assets: OnboardingState["assets"];
  nerModelDegraded: boolean;
  nerDownloadProgress: DownloadProgress | null;
  onComplete: () => void;
}) {
  return (
    <Onboarding
      assets={assets}
      nerModelDegraded={nerModelDegraded}
      nerDownloadProgress={nerDownloadProgress}
      onComplete={onComplete}
    />
  );
}
