import { render, screen, fireEvent } from "@testing-library/react";
import { describe, it, expect, vi } from "vitest";
import { DetectionModal } from "@/components/session/DetectionModal";
import { DEFAULT_SELECTED_CODES } from "@/lib/pii-categories";

describe("DetectionModal", () => {
  it("shows 5 sections and 14/27 count", () => {
    render(
      <DetectionModal open selection={new Set(DEFAULT_SELECTED_CODES)} onChange={() => {}} />,
    );
    expect(screen.getByText("DONNÉES PERSONNELLES")).toBeInTheDocument();
    expect(screen.getByText("DONNÉES D'ENTREPRISES")).toBeInTheDocument();
    expect(screen.getByText("DONNÉES DE LOCALISATION")).toBeInTheDocument();
    expect(screen.getByText("DONNÉES FINANCIÈRES")).toBeInTheDocument();
    expect(screen.getByText("DONNÉES DIVERSES")).toBeInTheDocument();
    expect(screen.getByText(/14 sur 27/)).toBeInTheDocument();
    // Header
    expect(screen.getByText("Données sélectionnées")).toBeInTheDocument();
    expect(screen.getByText(/Choisissez les types de données à pseudonymiser/)).toBeInTheDocument();
  });

  it("toggles checkbox on click", () => {
    const onChange = vi.fn();
    render(<DetectionModal open selection={new Set()} onChange={onChange} />);
    fireEvent.click(screen.getByLabelText("Nom de personne"));
    expect(onChange).toHaveBeenCalled();
  });

  it("footer has action buttons", () => {
    render(
      <DetectionModal open selection={new Set(DEFAULT_SELECTED_CODES)} onChange={() => {}} />,
    );
    expect(screen.getByText("Paramètres par défaut")).toBeInTheDocument();
    expect(screen.getAllByText("Tout sélectionner").length).toBeGreaterThanOrEqual(6);
    expect(screen.getAllByText("Tout désélectionner").length).toBeGreaterThanOrEqual(1);
  });

  it("each section has Tout sélectionner link", () => {
    render(
      <DetectionModal open selection={new Set(DEFAULT_SELECTED_CODES)} onChange={() => {}} />,
    );
    // 5 sections => 5 "Tout sélectionner" inside sections + 1 in footer = 6 at least
    const links = screen.getAllByText("Tout sélectionner");
    expect(links.length).toBeGreaterThanOrEqual(5);
  });

  it("shows 27 checkboxes", () => {
    render(<DetectionModal open selection={new Set()} onChange={() => {}} />);
    // Check that PR badge is rendered
    expect(screen.getByText("PR")).toBeInTheDocument();
    // Count checkboxes via role
    const checkboxes = screen.getAllByRole("checkbox");
    expect(checkboxes.length).toBe(27);
  });
});
