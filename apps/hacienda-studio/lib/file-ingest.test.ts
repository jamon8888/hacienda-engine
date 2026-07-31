import { describe, expect, it } from "vitest";
import { fileRelativePath, partitionFiles, formatSkippedMessage } from "./file-ingest";

function file(name: string, webkitRelativePath?: string): File {
  const f = new File(["content"], name, { type: "text/plain" });
  if (webkitRelativePath !== undefined) {
    Object.defineProperty(f, "webkitRelativePath", { value: webkitRelativePath });
  }
  return f;
}

describe("fileRelativePath (Track I1)", () => {
  it("returns the plain name for a flat (non-folder) selection", () => {
    expect(fileRelativePath(file("report.pdf"))).toBe("report.pdf");
  });

  it("returns webkitRelativePath when the file was selected via a folder input", () => {
    expect(fileRelativePath(file("report.pdf", "contracts/2024/report.pdf"))).toBe("contracts/2024/report.pdf");
  });

  it("falls back to name when webkitRelativePath is the empty string", () => {
    expect(fileRelativePath(file("report.pdf", ""))).toBe("report.pdf");
  });
});

describe("partitionFiles (Track I1)", () => {
  const alwaysValid = () => ({ valid: true });
  const rejectPdf = (f: File) =>
    f.name.endsWith(".pdf") ? { valid: true } : { valid: false, error: `Unsupported file type: ${f.type}` };

  it("puts every file in `valid` when the validator accepts them all", () => {
    const files = [file("a.pdf"), file("b.pdf")];

    const result = partitionFiles(files, alwaysValid);

    expect(result.valid).toEqual(files);
    expect(result.skipped).toEqual([]);
  });

  it("moves a rejected file to `skipped` with its relative path and reason, without dropping the rest of the batch", () => {
    const good = file("report.pdf", "contracts/report.pdf");
    const junk = file(".DS_Store", ".DS_Store");

    const result = partitionFiles([good, junk], rejectPdf);

    expect(result.valid).toEqual([good]);
    expect(result.skipped).toEqual([{ name: ".DS_Store", reason: "Unsupported file type: text/plain" }]);
  });
});

describe("formatSkippedMessage (Track I1)", () => {
  it("returns null when nothing was skipped", () => {
    expect(formatSkippedMessage([])).toBeNull();
  });

  it("names every file when 3 or fewer were skipped", () => {
    const skipped = [
      { name: "a.exe", reason: "Unsupported file type: application/x-msdownload" },
      { name: ".DS_Store", reason: "Unsupported file type: " },
    ];

    expect(formatSkippedMessage(skipped)).toBe(
      "a.exe: Unsupported file type: application/x-msdownload; .DS_Store: Unsupported file type: ",
    );
  });

  it("summarizes as a count plus a few examples when more than 3 were skipped, instead of listing every name", () => {
    const skipped = Array.from({ length: 5 }, (_, i) => ({ name: `junk-${i}`, reason: "Unsupported file type: " }));

    const message = formatSkippedMessage(skipped);

    expect(message).toBe("5 files skipped (unsupported): junk-0, junk-1, junk-2, and 2 more");
  });
});
