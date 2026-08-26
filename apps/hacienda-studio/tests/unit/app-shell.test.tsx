import { render, screen } from "@testing-library/react";
import { describe, it, expect } from "vitest";
import { AppShell } from "@/components/layout/AppShell";

describe("AppShell", () => {
  it("renders sidebar with Sessions active", () => {
    render(<AppShell sessions={[]} activeSession={null} />);
    // Sessions appears twice (sidebar + breadcrumb) — check at least one
    expect(screen.getAllByText("Sessions").length).toBeGreaterThan(0);
    expect(screen.getByText("Anonymisation")).toBeInTheDocument();
  });
  it("shows topbar tabs Vue d'ensemble / Éditeur / Pseudonymes / Restauration", () => {
    render(<AppShell sessions={[]} activeSession={null} />);
    expect(screen.getByText("Vue d'ensemble")).toBeInTheDocument();
    expect(screen.getByText("Restauration")).toBeInTheDocument();
  });
  it("shows tab Pseudonymes with count and extra sidebar sections", () => {
    render(<AppShell sessions={[]} activeSession={null} />);
    expect(screen.getByText("Pseudonymes")).toBeInTheDocument();
    expect(screen.getByText("Éditeur")).toBeInTheDocument();
    expect(screen.getByText("Accueil")).toBeInTheDocument();
    expect(screen.getByText("Intégrations")).toBeInTheDocument();
    expect(screen.getByText("Préférences")).toBeInTheDocument();
  });
  it("shows topbar action button", () => {
    render(<AppShell sessions={[]} activeSession={null} />);
    expect(screen.getByText(/S'inscrire et télécharger la session/)).toBeInTheDocument();
    expect(screen.getByText("Nouvelle session")).toBeInTheDocument();
  });
  it("shows breadcrumb Sessions / Session du 26/08", () => {
    render(<AppShell sessions={[]} activeSession={null} />);
    expect(screen.getByText(/Session du 26\/08/)).toBeInTheDocument();
  });
});
