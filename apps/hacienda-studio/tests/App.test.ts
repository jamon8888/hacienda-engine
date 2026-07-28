import { render, screen } from "@testing-library/svelte";
import { describe, it, expect, vi } from "vitest";
import App from "../src/App.svelte";

vi.mock("@xberg-io/xberg-wasm", () => ({
  init: vi.fn().mockResolvedValue(undefined),
  Xberg: vi.fn().mockImplementation(() => ({
    extract: vi.fn().mockResolvedValue({
      results: [
        {
          content: "# Test\n\nExtracted content",
          output_format: "markdown",
          metadata: {},
        },
      ],
    }),
  })),
}));

describe("App", () => {
  it("renders the Xberg Playground title", () => {
    render(App);
    expect(screen.getByText("Xberg Playground")).toBeInTheDocument();
  });

  it("shows loading state initially", () => {
    render(App);
    expect(
      screen.getByText("Loading Xberg WASM module..."),
    ).toBeInTheDocument();
  });

  it("shows file input after loading", async () => {
    render(App);
    await vi.waitFor(() => {
      expect(
        screen.getByLabelText("Select a file to extract"),
      ).toBeInTheDocument();
    });
  });
});
