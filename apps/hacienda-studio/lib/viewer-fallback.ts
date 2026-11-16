import { getViewerKind, type ViewerKind } from "./viewer-kind";

export interface ViewerPlan {
  kind: ViewerKind;
  usesMarkdownFallback: boolean;
  markdown: string;
  originalLink: {
    label: "Ouvrir l'original";
    available: boolean;
  };
}

/**
 * Chooses a human viewer without ever converting the original bytes to a browser link.
 * The caller supplies an already tokenized Markdown projection and a capability check
 * performed by the desktop/Tauri boundary.
 */
export function getViewerPlan(
  fileName: string,
  tokenizedMarkdown: string,
  options: { canOpenOriginal?: boolean } = {},
): ViewerPlan {
  const kind = getViewerKind(fileName);
  return {
    kind,
    usesMarkdownFallback: kind === null,
    markdown: tokenizedMarkdown,
    originalLink: {
      label: "Ouvrir l'original",
      available: Boolean(options.canOpenOriginal),
    },
  };
}
