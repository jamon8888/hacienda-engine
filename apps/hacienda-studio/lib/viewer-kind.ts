export type ViewerKind = "pdf" | "docx" | "xlsx" | "pptx" | null;

const EXTENSION_VIEWER_KIND: Record<string, ViewerKind> = {
  pdf: "pdf",
  docx: "docx",
  xlsx: "xlsx",
  pptx: "pptx",
};

/**
 * Extension-based, not `frontmatter.type` (MIME): the browser doesn't
 * reliably set `file.type` for Office formats on folder drag-drop. ~keep
 */
export function getViewerKind(name: string): ViewerKind {
  const match = /\.([^./]+)$/.exec(name);
  if (!match) return null;
  return EXTENSION_VIEWER_KIND[match[1].toLowerCase()] ?? null;
}
