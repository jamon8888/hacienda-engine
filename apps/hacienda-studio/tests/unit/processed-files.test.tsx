import { describe, it, expect } from "vitest";
import { render, screen } from "@testing-library/react";
import { ProcessedFilesList, type ProcessedFileItem } from "@/components/session/ProcessedFilesList";

const mockFiles: ReadonlyArray<ProcessedFileItem> = [
  { name: "[PSEUDONYMISÉ] Analyste-technique-de-test.pdf.md", subtitle: "2 pages · Aucune occurrence détectée", pseudonymized: true },
  { name: "[PSEUDONYMISÉ] hacienda-plan-implementation-agent-trust-layer.md", subtitle: "15 084 caractères · 22 occurrences détectées", pseudonymized: true },
  { name: "[PSEUDONYMISÉ] Befonts-License.txt", subtitle: "79 caractères · 1 occurrence détectée", pseudonymized: true },
];
const fileWith22: ProcessedFileItem = { name: "doc.md", subtitle: "15 084 caractères · 22 occurrences détectées" };

describe("ProcessedFilesList", () => {
  it("renders processed files with pseudonymized prefix", () => {
    render(<ProcessedFilesList files={mockFiles} onView={() => {}} onDelete={() => {}} />);
    expect(screen.getByText(/Analyste-technique-de-test/)).toBeInTheDocument();
    expect(screen.getByText(/Aucune occurrence détectée/)).toBeInTheDocument();
    expect(screen.getAllByText("Afficher").length).toBe(3);
  });
  it("shows occurrence count when >0", () => {
    render(<ProcessedFilesList files={[fileWith22]} onView={() => {}} onDelete={() => {}} />);
    expect(screen.getByText(/22 occurrences détectées/)).toBeInTheDocument();
  });
  it("renders Ajouter des documents secondary dropzone", () => {
    render(<ProcessedFilesList files={mockFiles} onView={() => {}} onDelete={() => {}} onAddFiles={() => {}} />);
    expect(screen.getByText(/Fichiers traités/)).toBeInTheDocument();
  });
});
