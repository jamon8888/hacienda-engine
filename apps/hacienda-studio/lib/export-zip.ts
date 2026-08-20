/**
 * Zips a set of processed documents' redacted markdown for download — shared between
 * the Documents library page (export selected/all) and the Studio queue page ("download
 * the whole folder" once a batch finishes), so both stay byte-identical instead of two
 * copies of the same JSZip call drifting apart.
 */
import type { ProcessedFile } from "./types";

export async function exportDocumentsZip(
  documents: { result: ProcessedFile }[],
  fileName?: string,
): Promise<void> {
  const JSZip = (await import("jszip")).default;
  const zip = new JSZip();
  for (const doc of documents) {
    zip.file(doc.result.name, doc.result.markdown);
  }
  const blob = await zip.generateAsync({ type: "blob" });
  const url = URL.createObjectURL(blob);
  const a = document.createElement("a");
  a.href = url;
  a.download = fileName ?? `hacienda-studio-export-${documents.length}.zip`;
  a.click();
  URL.revokeObjectURL(url);
}
