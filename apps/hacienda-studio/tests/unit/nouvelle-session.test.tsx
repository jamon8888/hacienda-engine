import { render, screen } from "@testing-library/react";
import { describe, it, expect } from "vitest";
import { Dropzone } from "@/components/session/Dropzone";
import { TreatmentPanel } from "@/components/session/TreatmentPanel";

describe("nouvelle session", () => {
  it("shows dropzone hint with limits", () => {
    render(<Dropzone onFiles={() => {}} />);
    expect(screen.getByText(/Cliquez ou glissez-déposez vos documents/)).toBeInTheDocument();
    expect(screen.getByText(/50 documents maximum/)).toBeInTheDocument();
  });
  it("TreatmentPanel shows Pseudonymiser active", () => {
    render(<TreatmentPanel mode="pseudonymize" onChange={() => {}} />);
    expect(screen.getByText("Pseudonymiser")).toBeInTheDocument();
    expect(screen.getByText("Anonymiser")).toBeInTheDocument();
  });
});
