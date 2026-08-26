import { describe, it, expect, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import { ProcessedFilesList } from "@/components/session/ProcessedFilesList";

const mockFiles = [
  { name: "[PSEUDONYMISÉ] Analyste-technique-de-test.pdf.md", subtitle: "2 pages · Aucune occurrence détectée", pseudonymized: true },
  { name: "[PSEUDONYMISÉ] hacienda-plan-implementation-agent-trust-layer.md", subtitle: "15 084 caractères · 22 occurrences détectées", pseudonymized: true },
  { name: "[PSEUDONYMISÉ] Befonts-License.txt", subtitle: "79 caractères · 1 occurrence détectée", pseudonymized: true },
];
const fileWith22 = { name: "doc.md", subtitle: "15 084 caractères · 22 occurrences détectées" };

describe("ProcessedFilesList", () => {
  it("renders processed files with pseudonymized prefix", () => {
    render(<ProcessedFilesList files={mockFiles as never} onView={() => {}} onDelete={() => {}} />);
    expect(screen.getByText(/Analyste-technique-de-test/)).toBeInTheDocument();
    expect(screen.getByText(/Aucune occurrence détectée/)).toBeInTheDocument();
    expect(screen.getAllByText("Afficher").length).toBe(3);
  });
  it("shows occurrence count when >0", () => {
    render(<ProcessedFilesList files={[fileWith22] as never} onView={() => {}} onDelete={() => {}} />);
    expect(screen.getByText(/22 occurrences détectées/)).toBeInTheDocument();
  });
  it("renders Ajouter des documents secondary dropzone", () => {
    render(<ProcessedFilesList files={mockFiles as never} onView={() => {}} onDelete={() => {}} onAddFiles={() => {}} />);
    expect(screen.getByText(/Fichiers traités/)).toBeInTheDocument();
  });
});
