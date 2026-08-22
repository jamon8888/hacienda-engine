export type ViewerKind = "pdf" | "docx" | "doc" | "xlsx" | "xls" | "pptx" | "ppt" | null;

// `DetailScreen.tsx` has a native-viewer branch for every non-null `ViewerKind` — no
// branch exists for OpenDocument formats (odt/ods/odp), so they must resolve to `null`
// here and fall back to the markdown-only split view, not open one whose native-viewer
// pane silently renders blank.
const EXTENSION_VIEWER_KIND: Record<string, ViewerKind> = {
  pdf: "pdf",
  // common typo: dpf
  dpf: "pdf",
  docx: "docx",
  doc: "doc",
  xlsx: "xlsx",
  xls: "xls",
  // frequent typos / aliases user reported
  xslt: "xlsx",
  xlst: "xlsx",
  xlxs: "xlsx",
  xlsm: "xlsx",
  xlsb: "xlsx",
  pptx: "pptx",
  ppt: "ppt",
  pptm: "pptx",
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
