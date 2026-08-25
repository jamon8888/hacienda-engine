/**
 * Task 1 (spec §8 step 2 / plan §1.3): `inferRelationships` used to assert `works_for`,
 * `partner_of`, and `contact_email` from bare co-occurrence — see the removed code's own
 * doc comment history and `registry.ts`'s current `inferRelationships` header for why.
 * These tests pin the replacement: proximity-scored `co_occurs_with` only, never a typed
 * employment/ownership/contact claim, and no O(n²) blowup on a large document.
 */
import { describe, it, expect } from "vitest";
import { BatchEntityRegistry } from "./registry";

/**
 * `BatchEntityRegistry.addEntity` is async (Task 4: it awaits a content hash for the
 * entity's id — see `registry.ts`'s `identityFor`). Every call in this file must be
 * awaited, not just for the type to check: `registerInDocument` (which populates
 * `docEntityMap`/`docEntitySpans`, what `inferRelationships` reads) only runs *after*
 * that await on a new entity's first registration. An un-awaited `addEntity` followed
 * by a synchronous `inferRelationships` call — the shape every test below uses — would
 * silently see an empty `docEntityMap` and emit zero relationships, for the wrong
 * reason entirely. This helper is `async` so a missing `await` at a call site is a type
 * error, not a silent race.
 */
async function addEntity(
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
  it("never emits a typed employment, ownership, or contact relation", async () => {
    const registry = new BatchEntityRegistry();
    const text = "Jean Dupont works at Acme SAS. Contact: jean@acme.example.";
    await addEntity(registry, "doc-001", "Jean Dupont", "person", [{ start: 0, end: 11 }]);
    await addEntity(registry, "doc-001", "Acme SAS", "organization", [{ start: 21, end: 29 }]);
    await addEntity(registry, "doc-001", "jean@acme.example", "email", [{ start: 41, end: 59 }]);

    registry.inferRelationships("doc-001", text);

    const types = new Set(registry.getRelationships().map((r) => r.relationship_type));
    expect(types.has("works_for")).toBe(false);
    expect(types.has("partner_of")).toBe(false);
    expect(types.has("contact_email")).toBe(false);
    for (const type of types) {
      expect(type).toBe("co_occurs_with");
    }
  });

  it("emits exactly one edge per unordered pair, not one per direction", async () => {
    const registry = new BatchEntityRegistry();
    const text = "Acme SAS and Beta Corp signed the agreement.";
    await addEntity(registry, "doc-001", "Acme SAS", "organization", [{ start: 0, end: 8 }]);
    await addEntity(registry, "doc-001", "Beta Corp", "organization", [{ start: 13, end: 22 }]);

    registry.inferRelationships("doc-001", text);

    expect(registry.getRelationships()).toHaveLength(1);
  });

  it("scores same-sentence proximity higher than same-paragraph", async () => {
    const registrySameSentence = new BatchEntityRegistry();
    const sentenceText = "Jean Dupont met Marie Curie at the conference.";
    await addEntity(registrySameSentence, "doc-001", "Jean Dupont", "person", [{ start: 0, end: 11 }]);
    await addEntity(registrySameSentence, "doc-001", "Marie Curie", "person", [{ start: 16, end: 27 }]);
    registrySameSentence.inferRelationships("doc-001", sentenceText);
    const sameSentenceConfidence = registrySameSentence.getRelationships()[0].confidence;

    const registryFarther = new BatchEntityRegistry();
    const paragraphText =
      "Jean Dupont opened the session. He thanked the organizers at length. " +
      "Marie Curie then took the floor.";
    await addEntity(registryFarther, "doc-001", "Jean Dupont", "person", [{ start: 0, end: 11 }]);
    await addEntity(registryFarther, "doc-001", "Marie Curie", "person", [
      { start: paragraphText.indexOf("Marie Curie"), end: paragraphText.indexOf("Marie Curie") + 11 },
    ]);
    registryFarther.inferRelationships("doc-001", paragraphText);
    const sameParagraphConfidence = registryFarther.getRelationships()[0].confidence;

    expect(sameSentenceConfidence).toBeGreaterThan(sameParagraphConfidence);
  });

  it("emits no edge across a paragraph break", async () => {
    const registry = new BatchEntityRegistry();
    const text = "Jean Dupont opened the session.\n\nMarie Curie closed it.";
    await addEntity(registry, "doc-001", "Jean Dupont", "person", [{ start: 0, end: 11 }]);
    await addEntity(registry, "doc-001", "Marie Curie", "person", [
      { start: text.indexOf("Marie Curie"), end: text.indexOf("Marie Curie") + 11 },
    ]);

    registry.inferRelationships("doc-001", text);

    expect(registry.getRelationships()).toHaveLength(0);
  });

  it("does not explode on a large document: entities spaced past the proximity window get no edge at all", async () => {
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
      await addEntity(registry, "doc-001", name, type, [{ start, end: start + name.length }]);
    }

    registry.inferRelationships("doc-001", text);

    expect(registry.getRelationships()).toHaveLength(0);
  });

  it("registers a repeat appearance of an entity in a later document (Task 1 bug fix)", async () => {
    const registry = new BatchEntityRegistry();
    await addEntity(registry, "doc-001", "Acme SAS", "organization", [{ start: 0, end: 8 }]);
    const entity = await addEntity(registry, "doc-002", "Acme SAS", "organization", [{ start: 5, end: 13 }]);

    expect(entity.source_documents).toEqual(["doc-001", "doc-002"]);
    expect(entity.mention_count).toBe(2);

    // Proof the fix is load-bearing, not just source_documents bookkeeping: doc-002's
    // inferRelationships must be able to see this entity to score anything against it.
    const text = "        Acme SAS confirmed the order.";
    await addEntity(registry, "doc-002", "the order", "organization", [{ start: 30, end: 39 }]);
    registry.inferRelationships("doc-002", text);
    expect(registry.getRelationships().length).toBeGreaterThan(0);
  });

  it("emits nothing when text is omitted", async () => {
    const registry = new BatchEntityRegistry();
    await addEntity(registry, "doc-001", "Acme SAS", "organization", [{ start: 0, end: 8 }]);
    await addEntity(registry, "doc-001", "Beta Corp", "organization", [{ start: 13, end: 22 }]);

    registry.inferRelationships("doc-001");

    expect(registry.getRelationships()).toHaveLength(0);
  });
});

describe("BatchEntityRegistry person alias matching", () => {
  it("merges a bare honorific mention into the full name seen first", async () => {
    const registry = new BatchEntityRegistry();
    await addEntity(registry, "doc-001", "Jean Dupont", "person", [{ start: 0, end: 11 }]);
    const entity = await addEntity(registry, "doc-002", "M. Dupont", "person", [{ start: 0, end: 9 }]);

    expect(registry.getEntities()).toHaveLength(1);
    expect(entity.display_name).toBe("Jean Dupont");
    expect(entity.aliases).toContain("M. Dupont");
    expect(entity.source_documents).toEqual(["doc-001", "doc-002"]);
  });

  it("promotes the canonical name once a fuller form is seen after a bare one", async () => {
    const registry = new BatchEntityRegistry();
    await addEntity(registry, "doc-001", "M. Dupont", "person", [{ start: 0, end: 9 }]);
    const entity = await addEntity(registry, "doc-002", "Jean Dupont", "person", [{ start: 0, end: 11 }]);

    expect(registry.getEntities()).toHaveLength(1);
    expect(entity.display_name).toBe("Jean Dupont");
    expect(entity.aliases).toContain("M. Dupont");
  });

  it("does not change id/slug when a fuller name is promoted to canonical (Task 4 stability)", async () => {
    // The reconciliation between this alias-matching feature and Task 4's stable-id work
    // fixed a real bug here: promotion used to recompute `slug` from the newly-canonical
    // surface form (`slugify(surfaceForm)`), which would silently change an entity's
    // filename the moment a fuller name was seen — exactly the re-export instability
    // Task 4 exists to eliminate. id/slug must stay whatever they were at creation.
    const registry = new BatchEntityRegistry();
    const bare = await addEntity(registry, "doc-001", "M. Dupont", "person", [{ start: 0, end: 9 }]);
    const promoted = await addEntity(registry, "doc-002", "Jean Dupont", "person", [{ start: 0, end: 11 }]);

    expect(promoted.id).toBe(bare.id);
    expect(promoted.slug).toBe(bare.slug);
  });

  it("does not merge two different people sharing a surname", async () => {
    const registry = new BatchEntityRegistry();
    await addEntity(registry, "doc-001", "Jean Dupont", "person", [{ start: 0, end: 11 }]);
    await addEntity(registry, "doc-002", "Marie Dupont", "person", [{ start: 0, end: 12 }]);

    expect(registry.getEntities()).toHaveLength(2);
  });

  it("leaves a bare honorific unmerged when two full names could match it", async () => {
    const registry = new BatchEntityRegistry();
    await addEntity(registry, "doc-001", "Jean Dupont", "person", [{ start: 0, end: 11 }]);
    await addEntity(registry, "doc-002", "Marie Dupont", "person", [{ start: 0, end: 12 }]);
    await addEntity(registry, "doc-003", "M. Dupont", "person", [{ start: 0, end: 9 }]);

    expect(registry.getEntities()).toHaveLength(3);
  });

  it("does not merge different surnames even with a shared given name", async () => {
    const registry = new BatchEntityRegistry();
    await addEntity(registry, "doc-001", "Jean Dupont", "person", [{ start: 0, end: 11 }]);
    await addEntity(registry, "doc-002", "Jean Martin", "person", [{ start: 0, end: 11 }]);

    expect(registry.getEntities()).toHaveLength(2);
  });

  it("does not apply alias matching to non-person entities", async () => {
    const registry = new BatchEntityRegistry();
    await addEntity(registry, "doc-001", "Acme Corporation", "organization", [{ start: 0, end: 16 }]);
    await addEntity(registry, "doc-002", "Corporation", "organization", [{ start: 0, end: 11 }]);

    expect(registry.getEntities()).toHaveLength(2);
  });
});

describe("BatchEntityRegistry identity and aliasing (Task 4, spec §8 step 4)", () => {
  it("gives the same entity the same id and slug regardless of which document order it was registered in", async () => {
    const registryA = new BatchEntityRegistry();
    await addEntity(registryA, "doc-001", "Jean Dupont", "person", [{ start: 0, end: 11 }]);
    const acmeA = await addEntity(registryA, "doc-002", "Acme SAS", "organization", [{ start: 0, end: 8 }]);

    const registryB = new BatchEntityRegistry();
    // Same two entities, opposite document order — simulates re-exporting the same
    // corpus with a different upload order, the exact instability this task fixes.
    const acmeB = await addEntity(registryB, "doc-001", "Acme SAS", "organization", [{ start: 0, end: 8 }]);
    await addEntity(registryB, "doc-002", "Jean Dupont", "person", [{ start: 0, end: 11 }]);

    expect(acmeA.id).toBe(acmeB.id);
    expect(acmeA.slug).toBe(acmeB.slug);
  });

  it("merges 'Acme', 'Acme SAS', and 'ACME S.A.S.' into one entity, with the longest form as display_name", async () => {
    const registry = new BatchEntityRegistry();
    await addEntity(registry, "doc-001", "Acme", "organization", [{ start: 0, end: 4 }]);
    await addEntity(registry, "doc-002", "Acme SAS", "organization", [{ start: 0, end: 8 }]);
    const merged = await addEntity(registry, "doc-003", "ACME S.A.S.", "organization", [
      { start: 0, end: 11 },
    ]);

    expect(registry.getEntities()).toHaveLength(1);
    expect(merged.display_name).toBe("ACME S.A.S.");
    expect(merged.aliases.sort()).toEqual(["Acme", "Acme SAS"]);
    expect(merged.source_documents).toEqual(["doc-001", "doc-002", "doc-003"]);
  });

  it("does not merge a genuinely distinct organisation whose name happens to share a prefix", async () => {
    const registry = new BatchEntityRegistry();
    // Near-miss pair: "Acme SAS" and "Acme Global SAS" share the "Acme"/"SAS" wrapping,
    // but "Global" is real distinguishing content, not a legal-form suffix — the two
    // must stay separate entities, not collapse the way "Acme"/"Acme SAS" correctly do.
    await addEntity(registry, "doc-001", "Acme SAS", "organization", [{ start: 0, end: 8 }]);
    await addEntity(registry, "doc-001", "Acme Global SAS", "organization", [{ start: 20, end: 35 }]);

    expect(registry.getEntities()).toHaveLength(2);
    const names = registry.getEntities().map((e) => e.display_name).sort();
    expect(names).toEqual(["Acme Global SAS", "Acme SAS"]);
  });

  it("does not apply legal-suffix stripping to non-organisation entities", async () => {
    const registry = new BatchEntityRegistry();
    // "Sa" is a real (if unusual) surname fragment; legal-suffix stripping must be
    // scoped to organizations only, or a person named "... Sa" would wrongly merge
    // with an unrelated person whose name happens to end the same way.
    await addEntity(registry, "doc-001", "Jean Sa", "person", [{ start: 0, end: 7 }]);
    await addEntity(registry, "doc-001", "Marie Sa", "person", [{ start: 20, end: 28 }]);

    expect(registry.getEntities()).toHaveLength(2);
  });
});
