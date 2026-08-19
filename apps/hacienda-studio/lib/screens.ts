/**
 * The five first-class screens of the app's flow: asset loading → upload → processing
 * queue → file browser → split-view detail. Replaces the prior pair of loosely-coupled
 * state fields (`onboardingComplete: boolean`, `openResultName: string | null`) that let
 * screens coexist via conditional rendering rather than real navigation — `detail.inputName`
 * is now the single source of truth for "which file is open," the same role
 * `openResultName` played, but scoped inside the one state value that also encodes which
 * screen is showing.
 */
export type Screen =
  | { kind: "asset-loading" }
  | { kind: "upload" }
  | { kind: "queue" }
  | { kind: "browser" }
  // `inputName` = `ProcessedFile.frontmatter.source`, the original input file name —
  // not `ProcessedFile.name` (the `.md` output name). Same semantics as the old
  // `openResultName`, which this replaces.
  | { kind: "detail"; inputName: string };

export function isDetailFor(screen: Screen, inputName: string): boolean {
  return screen.kind === "detail" && screen.inputName === inputName;
}
