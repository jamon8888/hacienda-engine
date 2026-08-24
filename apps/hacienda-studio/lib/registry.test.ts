/**
 * Task 1 (spec §8 step 2 / plan §1.3): `inferRelationships` used to assert `works_for`,
 * `partner_of`, and `contact_email` from bare co-occurrence — see the removed code's own
 * doc comment history and `registry.ts`'s current `inferRelationships` header for why.
 * These tests pin the replacement: proximity-scored `co_occurs_with` only, never a typed
 * employment/ownership/contact claim, and no O(n²) blowup on a large document.
 */
import { describe, it, expect } from "vitest";
import { BatchEntityRegistry } from "./registry";

function addEntity(
  registry: BatchEntityRegistry,
  docId: string,
  name: string,
  type: string,
  spans: Array<{ start: number; end: number }>,
) {
  return registry.addEntity(
    { name, type, slug: name.toLowerCase().replace(/\s+/g, "-"), count: spans.length, spans },
    { vertical: "shared" },
    docId,
  );
}

describe("BatchEntityRegistry.inferRelationships", () => {
  it("never emits a typed employment, ownership, or contact relation", () => {
    const registry = new BatchEntityRegistry();
    const text = "Jean Dupont works at Acme SAS. Contact: jean@acme.example.";
    addEntity(registry, "doc-001", "Jean Dupont", "person", [{ start: 0, end: 11 }]);
    addEntity(registry, "doc-001", "Acme SAS", "organization", [{ start: 21, end: 29 }]);
    addEntity(registry, "doc-001", "jean@acme.example", "email", [{ start: 41, end: 59 }]);

    registry.inferRelationships("doc-001", text);

    const types = new Set(registry.getRelationships().map((r) => r.relationship_type));
    expect(types.has("works_for")).toBe(false);
    expect(types.has("partner_of")).toBe(false);
    expect(types.has("contact_email")).toBe(false);
    for (const type of types) {
      expect(type).toBe("co_occurs_with");
    }
  });

  it("emits exactly one edge per unordered pair, not one per direction", () => {
    const registry = new BatchEntityRegistry();
    const text = "Acme SAS and Beta Corp signed the agreement.";
    addEntity(registry, "doc-001", "Acme SAS", "organization", [{ start: 0, end: 8 }]);
    addEntity(registry, "doc-001", "Beta Corp", "organization", [{ start: 13, end: 22 }]);

    registry.inferRelationships("doc-001", text);

    expect(registry.getRelationships()).toHaveLength(1);
  });

  it("scores same-sentence proximity higher than same-paragraph", () => {
    const registrySameSentence = new BatchEntityRegistry();
    const sentenceText = "Jean Dupont met Marie Curie at the conference.";
    addEntity(registrySameSentence, "doc-001", "Jean Dupont", "person", [{ start: 0, end: 11 }]);
    addEntity(registrySameSentence, "doc-001", "Marie Curie", "person", [{ start: 16, end: 27 }]);
    registrySameSentence.inferRelationships("doc-001", sentenceText);
    const sameSentenceConfidence = registrySameSentence.getRelationships()[0].confidence;

    const registryFarther = new BatchEntityRegistry();
    const paragraphText =
      "Jean Dupont opened the session. He thanked the organizers at length. " +
      "Marie Curie then took the floor.";
    addEntity(registryFarther, "doc-001", "Jean Dupont", "person", [{ start: 0, end: 11 }]);
    addEntity(registryFarther, "doc-001", "Marie Curie", "person", [
      { start: paragraphText.indexOf("Marie Curie"), end: paragraphText.indexOf("Marie Curie") + 11 },
    ]);
    registryFarther.inferRelationships("doc-001", paragraphText);
    const sameParagraphConfidence = registryFarther.getRelationships()[0].confidence;

    expect(sameSentenceConfidence).toBeGreaterThan(sameParagraphConfidence);
  });

  it("emits no edge across a paragraph break", () => {
    const registry = new BatchEntityRegistry();
    const text = "Jean Dupont opened the session.\n\nMarie Curie closed it.";
    addEntity(registry, "doc-001", "Jean Dupont", "person", [{ start: 0, end: 11 }]);
    addEntity(registry, "doc-001", "Marie Curie", "person", [
      { start: text.indexOf("Marie Curie"), end: text.indexOf("Marie Curie") + 11 },
    ]);

    registry.inferRelationships("doc-001", text);

    expect(registry.getRelationships()).toHaveLength(0);
  });

  it("does not explode on a large document: entities spaced past the proximity window get no edge at all", () => {
    // The failure mode this guards against is structural, not a magic count: a blank-line-
    // only proximity check (no distance cap) treats an entire multi-page paragraph as one
    // "close together" blob, so 40 entities anywhere in it produce all C(40,2)=780 pairs as
    // edges — confirmed by running this exact scenario before the distance cap was added.
    // Spacing every mention well past `classifyProximity`'s cap, with no blank lines
    // anywhere, isolates that: if the cap is doing its job, this must produce **zero**
    // edges regardless of entity count, because "far enough apart" doesn't depend on n.
    const registry = new BatchEntityRegistry();
    const filler =
      "In the intervening pages, considerable additional prose discusses unrelated matters " +
      "at length so that no two named parties are ever mentioned near one another again ";
    let text = "";
    const names: Array<{ name: string; type: string }> = [];
    for (let i = 0; i < 40; i++) names.push({ name: `Person${i} Lastname${i}`, type: "person" });
    for (let i = 0; i < 5; i++) names.push({ name: `Org${i} Holdings`, type: "organization" });

    for (const { name } of names) {
      text += `${name} is mentioned here. `;
      const target = Math.ceil(text.length / 500) * 500 + 500; // pad well past the 300-char cap
      while (text.length < target) text += filler;
      text = text.slice(0, target);
    }

    for (const { name, type } of names) {
      const start = text.indexOf(name);
      addEntity(registry, "doc-001", name, type, [{ start, end: start + name.length }]);
    }

    registry.inferRelationships("doc-001", text);

    expect(registry.getRelationships()).toHaveLength(0);
  });

  it("registers a repeat appearance of an entity in a later document (Task 1 bug fix)", () => {
    const registry = new BatchEntityRegistry();
    addEntity(registry, "doc-001", "Acme SAS", "organization", [{ start: 0, end: 8 }]);
    const entity = addEntity(registry, "doc-002", "Acme SAS", "organization", [{ start: 5, end: 13 }]);

    expect(entity.source_documents).toEqual(["doc-001", "doc-002"]);
    expect(entity.mention_count).toBe(2);

    // Proof the fix is load-bearing, not just source_documents bookkeeping: doc-002's
    // inferRelationships must be able to see this entity to score anything against it.
    const text = "        Acme SAS confirmed the order.";
    addEntity(registry, "doc-002", "the order", "organization", [{ start: 30, end: 39 }]);
    registry.inferRelationships("doc-002", text);
    expect(registry.getRelationships().length).toBeGreaterThan(0);
  });

  it("emits nothing when text is omitted", () => {
    const registry = new BatchEntityRegistry();
    addEntity(registry, "doc-001", "Acme SAS", "organization", [{ start: 0, end: 8 }]);
    addEntity(registry, "doc-001", "Beta Corp", "organization", [{ start: 13, end: 22 }]);

    registry.inferRelationships("doc-001");

    expect(registry.getRelationships()).toHaveLength(0);
  });
});

describe("BatchEntityRegistry person alias matching", () => {
  it("merges a bare honorific mention into the full name seen first", () => {
    const registry = new BatchEntityRegistry();
    addEntity(registry, "doc-001", "Jean Dupont", "person", [{ start: 0, end: 11 }]);
    const entity = addEntity(registry, "doc-002", "M. Dupont", "person", [{ start: 0, end: 9 }]);

    expect(registry.getEntities()).toHaveLength(1);
    expect(entity.display_name).toBe("Jean Dupont");
    expect(entity.aliases).toContain("M. Dupont");
    expect(entity.source_documents).toEqual(["doc-001", "doc-002"]);
  });

  it("promotes the canonical name once a fuller form is seen after a bare one", () => {
    const registry = new BatchEntityRegistry();
    addEntity(registry, "doc-001", "M. Dupont", "person", [{ start: 0, end: 9 }]);
    const entity = addEntity(registry, "doc-002", "Jean Dupont", "person", [{ start: 0, end: 11 }]);

    expect(registry.getEntities()).toHaveLength(1);
    expect(entity.display_name).toBe("Jean Dupont");
    expect(entity.aliases).toContain("M. Dupont");
  });

  it("does not merge two different people sharing a surname", () => {
    const registry = new BatchEntityRegistry();
    addEntity(registry, "doc-001", "Jean Dupont", "person", [{ start: 0, end: 11 }]);
    addEntity(registry, "doc-002", "Marie Dupont", "person", [{ start: 0, end: 12 }]);

    expect(registry.getEntities()).toHaveLength(2);
  });

  it("leaves a bare honorific unmerged when two full names could match it", () => {
    const registry = new BatchEntityRegistry();
    addEntity(registry, "doc-001", "Jean Dupont", "person", [{ start: 0, end: 11 }]);
    addEntity(registry, "doc-002", "Marie Dupont", "person", [{ start: 0, end: 12 }]);
    addEntity(registry, "doc-003", "M. Dupont", "person", [{ start: 0, end: 9 }]);

    expect(registry.getEntities()).toHaveLength(3);
  });

  it("does not merge different surnames even with a shared given name", () => {
    const registry = new BatchEntityRegistry();
    addEntity(registry, "doc-001", "Jean Dupont", "person", [{ start: 0, end: 11 }]);
    addEntity(registry, "doc-002", "Jean Martin", "person", [{ start: 0, end: 11 }]);

    expect(registry.getEntities()).toHaveLength(2);
  });

  it("does not apply alias matching to non-person entities", () => {
    const registry = new BatchEntityRegistry();
    addEntity(registry, "doc-001", "Acme Corporation", "organization", [{ start: 0, end: 16 }]);
    addEntity(registry, "doc-002", "Corporation", "organization", [{ start: 0, end: 11 }]);

    expect(registry.getEntities()).toHaveLength(2);
  });
});
