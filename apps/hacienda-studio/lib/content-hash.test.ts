import { describe, it, expect } from "vitest";
import { computeContentHash } from "./content-hash";

describe("computeContentHash", () => {
  it("is deterministic for identical bytes", async () => {
    const bytes = new TextEncoder().encode("hello world").buffer;
    const a = await computeContentHash(bytes);
    const b = await computeContentHash(bytes);
    expect(a).toBe(b);
  });

  it("differs for different bytes", async () => {
    const a = await computeContentHash(new TextEncoder().encode("hello").buffer);
    const b = await computeContentHash(new TextEncoder().encode("world").buffer);
    expect(a).not.toBe(b);
  });

  it("returns a 64-character lowercase hex string (SHA-256)", async () => {
    const hash = await computeContentHash(new TextEncoder().encode("x").buffer);
    expect(hash).toMatch(/^[0-9a-f]{64}$/);
  });
});
