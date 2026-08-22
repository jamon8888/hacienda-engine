/**
 * Coarse device-capability tier used to scale the pipeline's RAM-heavy choices
 * (worker pool size, whether to load the ~600MB neural NER model at all) to what the
 * client's browser actually has available.
 *
 * `navigator.deviceMemory` (Chrome/Edge only, `undefined` on Firefox/Safari) reports
 * an approximate figure the spec quantizes to {0.25, 0.5, 1, 2, 4, 8} — any machine
 * with 8GB or more reports exactly `8`, never higher. That cap means an 8GB machine
 * is indistinguishable from a 64GB one by `deviceMemory` alone. Each pool worker
 * holds its own ~600MB NER model plus its own wasm heap (`lib/worker-pool.ts`), so
 * naively treating every `mem === 8` machine as "plenty of headroom" and handing it a
 * 3-worker pool (~1.8GB of NER models alone, before wasm/OCR/browser overhead) is
 * exactly what OOM-freezes an 8GB laptop. `hardwareConcurrency` (logical core count)
 * is used as a secondary signal to split that ambiguous case: real high-RAM desktops
 * (>8GB) also tend to have more cores than a typical 8GB laptop, so only bump to
 * "high" when both signals agree.
 */
export type DeviceTier = "low" | "medium" | "high";

const HIGH_TIER_MIN_CORES = 8;

export function detectDeviceTier(): DeviceTier {
  const mem = (navigator as unknown as { deviceMemory?: number }).deviceMemory;
  const cores = navigator.hardwareConcurrency ?? 4;
  if (mem !== undefined) {
    if (mem <= 2) return "low";
    if (mem <= 4) return "medium";
    // mem === 8 is the spec's ceiling and covers an 8GB machine same as a much
    // larger one — only trust it as "high" when core count corroborates it.
    return cores >= HIGH_TIER_MIN_CORES ? "high" : "medium";
  }
  if (cores <= 2) return "low";
  if (cores <= 4) return "medium";
  return "high";
}

/**
 * Worker pool size per tier — mirrors `lib/worker-pool.ts`'s own "~600MB/worker"
 * comment. `3` was previously the flat default regardless of tier; now only `high`
 * (e.g. an 8GB+ machine) gets it, `medium` and `low` scale down so a smaller machine
 * doesn't try to hold multiple 600MB models plus wasm heaps plus the browser/OS.
 */
export function poolSizeForTier(tier: DeviceTier): number {
  switch (tier) {
    case "low":
      return 1;
    case "medium":
      return 2;
    case "high":
      return 3;
  }
}
