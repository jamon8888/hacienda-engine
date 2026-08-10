export type ViewerKind = "pdf" | "docx" | "doc" | "xlsx" | "xls" | "pptx" | "ppt" | "odt" | "ods" | "odp" | null;

const EXTENSION_VIEWER_KIND: Record<string, ViewerKind> = {
  pdf: "pdf",
  docx: "docx",
  doc: "doc",
  xlsx: "xlsx",
  xls: "xls",
  pptx: "pptx",
  ppt: "ppt",
  odt: "odt",
  ods: "ods",
  odp: "odp",
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
