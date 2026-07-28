import { describe, it, expect, vi } from "vitest";
import { render, screen } from "@testing-library/svelte";
import Onboarding from "./Onboarding.svelte";

describe("Onboarding", () => {
  it("shows local AI messaging", () => {
    render(Onboarding, {
      props: {
        assets: { xbergWasm: true, nerModel: true, tessdata: true },
        onComplete: vi.fn(),
      },
    });
    expect(screen.getByText(/100% local/i)).toBeInTheDocument();
    expect(screen.getByText(/never leave/i)).toBeInTheDocument();
  });

  it("shows progress for each asset", () => {
    render(Onboarding, {
      props: {
        assets: { xbergWasm: true, nerModel: false, tessdata: true },
        onComplete: vi.fn(),
      },
    });
    expect(screen.getByText(/ner model/i)).toBeInTheDocument();
  });

  it("disables continue until all assets cached", () => {
    render(Onboarding, {
      props: {
        assets: { xbergWasm: true, nerModel: false, tessdata: true },
        onComplete: vi.fn(),
      },
    });
    expect(screen.getByRole("button", { name: /continue/i })).toBeDisabled();
  });
});
