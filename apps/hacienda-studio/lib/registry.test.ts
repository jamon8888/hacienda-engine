import { describe, expect, it } from "vitest";
import { BatchEntityRegistry } from "./registry";

function entityInput(name: string, type = "organization") {
  return { name, type, slug: "unused", count: 1, spans: [] };
}

describe("BatchEntityRegistry slug assignment (Track I2)", () => {
  it("assigns a slug derived from the entity's name, ignoring the caller-supplied slug", () => {
    const registry = new BatchEntityRegistry();

    const result = registry.addEntity(entityInput("Acme SAS"), { vertical: "m&a" }, "doc-001");

    expect(result.slug).toBe("acme-sas");
  });

  it("returns the same slug for repeated mentions of the same entity", () => {
    const registry = new BatchEntityRegistry();

    const first = registry.addEntity(entityInput("Acme SAS"), { vertical: "m&a" }, "doc-001");
    const second = registry.addEntity(entityInput("Acme SAS"), { vertical: "m&a" }, "doc-002");

    expect(second.id).toBe(first.id);
    expect(second.slug).toBe(first.slug);
  });

  /**
   * Regression target for `entities/<slug>.md`: two distinct entities (different
   * type, so they don't dedupe to the same registry row) that happen to slugify
   * to the same base string must not silently overwrite each other's file in the
   * exported vault.
   */
  it("resolves a slug collision between two distinct entities by suffixing", () => {
    const registry = new BatchEntityRegistry();

    const org = registry.addEntity(entityInput("Acme SAS", "organization"), { vertical: "m&a" }, "doc-001");
    const person = registry.addEntity(entityInput("Acme, SAS.", "person"), { vertical: "m&a" }, "doc-001");

    expect(org.slug).toBe("acme-sas");
    expect(person.slug).toBe("acme-sas-2");
    expect(person.id).not.toBe(org.id);
  });

  it("falls back to entity-<id> when the name slugifies to an empty string", () => {
    const registry = new BatchEntityRegistry();

    const result = registry.addEntity(entityInput("***"), { vertical: "shared" }, "doc-001");

    expect(result.slug).toBe(`entity-${result.id}`);
  });
});
