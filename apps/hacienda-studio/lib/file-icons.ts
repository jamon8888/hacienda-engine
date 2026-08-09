/**
 * Track K3 (file-browser type icons): reuses `@pierre/trees`' own icon resolver and SVG
 * sprite sheet — the same lookup `components/extend/file-system.tsx` already vendors for
 * its full tree/gallery views — instead of hand-rolling a second file-extension-to-icon
 * map. Monochrome only (no per-token color palette): `file-system.tsx`'s ~50-entry
 * light/dark color table exists for its own themed grid/gallery views, but this app has no
 * app-wide dark mode (see `App.tsx`'s single viewer-scoped `viewerDark` toggle) — icons here
 * render in `currentColor`, same as every other muted list glyph in `FileBrowser.tsx`.
 */
import { createFileTreeIconResolver, getBuiltInSpriteSheet } from "@pierre/trees";

export const FILE_ICON_SPRITE_SHEET = getBuiltInSpriteSheet("complete");

const { resolveIcon } = createFileTreeIconResolver({ set: "complete", colored: false });

export function resolveFileIcon(fileName: string): ReturnType<typeof resolveIcon> {
  return resolveIcon("file-tree-icon-file", fileName);
}
