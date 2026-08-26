import { describe, it, expect } from "vitest";
import { createSession } from "@/lib/session";
import { DETECTION_CATEGORIES, categoryToWire } from "@/lib/pii-categories";
import { conversionToWasmFormat } from "@/lib/conversion";

describe("session model", () => {
  it("creates session with default 14 selections matching screenshot", () => {
    const s = createSession("Session du 26/08");
    expect(s.detectionSelection.size).toBe(14);
    expect(DETECTION_CATEGORIES.length).toBe(5);
  });
  it("maps PR -> person, MAIL -> email, PHON -> phone_number", () => {
    expect(categoryToWire("PR")).toBe("person");
    expect(categoryToWire("MAIL")).toBe("email");
    expect(categoryToWire("PHON")).toBe("phone_number");
  });
  it("conversion markdown -> WasmOutputFormat.Markdown", () => {
    expect(conversionToWasmFormat("markdown")).toBe("markdown");
  });
});
