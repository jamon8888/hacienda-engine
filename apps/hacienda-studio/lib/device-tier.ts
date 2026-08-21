/**
 * Coarse device-capability tier used to scale the pipeline's RAM-heavy choices
 * (worker pool size, whether to load the ~600MB neural NER model at all) to what the
 * client's browser actually has available.
 *
 * `navigator.deviceMemory` (Chrome/Edge only, `undefined` on Firefox/Safari) reports
 * an approximate figure the spec quantizes to {0.25, 0.5, 1, 2, 4, 8} — any machine
 * with 8GB or more reports exactly `8`, never higher. That cap is fine for a coarse
 * tier (we only need "can this comfortably hold a 600MB model", not an exact number),
 * but it does mean 8GB and 64GB machines land in the same tier.
 *
 * When `deviceMemory` is unavailable, `navigator.hardwareConcurrency` (logical core
 * count, broadly supported) is used as a secondary signal — low core count usually
 * correlates with lower-end hardware.
 */
export type DeviceTier = "low" | "medium" | "high";

export function detectDeviceTier(): DeviceTier {
  const mem = (navigator as unknown as { deviceMemory?: number }).deviceMemory;
  if (mem !== undefined) {
    if (mem <= 2) return "low";
    if (mem <= 4) return "medium";
    return "high"; // 8 is the spec's ceiling — covers an 8GB machine same as higher
  }
  const cores = navigator.hardwareConcurrency ?? 4;
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
