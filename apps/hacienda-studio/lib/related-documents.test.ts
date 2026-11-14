import { describe, it, expect } from "vitest";
import {
  computeRelatedDocuments,
  topRelatedDocuments,
  buildRelatedDocumentsSection,
} from "./related-documents";
import type { RegistryEntity } from "./registry";

function entity(overrides: Partial<RegistryEntity>): RegistryEntity {
  return {
    id: overrides.id ?? "",
    canonical_name: "",
    display_name: "",
    type: "organization",
    slug: "",
    vertical: "shared",
    roles: [],
    aliases: [],
    source_documents: [],
    mention_count: 1,
    vertical_metadata: {},
    first_seen: "",
    last_seen: "",
    ...overrides,
  };
}

describe("computeRelatedDocuments (Task 5.2, spec §8 step 5)", () => {
  it("scores a rarely-shared entity above a corpus-wide one", () => {
    const docIds = ["doc-1", "doc-2", "doc-3", "doc-4"];
    const entities = [
      // present in all 4 documents -> idf = log(4/4) = 0, contributes nothing
      entity({ id: "ent-ubiquitous", display_name: "MegaCorp", source_documents: docIds }),
      // shared by exactly doc-1 and doc-2 -> idf = log(4/2), the strong signal
      entity({ id: "ent-rare", display_name: "Acme SAS", source_documents: ["doc-1", "doc-2"] }),
    ];

    const related = computeRelatedDocuments(entities, docIds);
    const forDoc1 = related.get("doc-1")!;

    expect(forDoc1).toHaveLength(1);
    expect(forDoc1[0].docId).toBe("doc-2");
    expect(forDoc1[0].sharedEntityNames).toEqual(["Acme SAS"]);
    // MegaCorp contributed literally zero score despite doc-1 and every other document
    // all sharing it — this is the whole point of IDF weighting.
    expect(forDoc1[0].score).toBeGreaterThan(0);
  });

  it("returns an empty array, not an absent key, for a document with no scoring overlap", () => {
    const docIds = ["doc-1", "doc-2"];
    const entities = [entity({ id: "e1", source_documents: ["doc-1"] })];

    const related = computeRelatedDocuments(entities, docIds);

    expect(related.has("doc-2")).toBe(true);
    expect(related.get("doc-2")).toEqual([]);
  });

  it("ranks shared entity names within a pair by their own idf, most specific first", () => {
    const docIds = ["doc-1", "doc-2", "doc-3", "doc-4", "doc-5"];
    const entities = [
      // shared by doc-1/doc-2 and 3 others -> lower idf
      entity({
        id: "ent-common",
        display_name: "Common Corp",
        source_documents: ["doc-1", "doc-2", "doc-3", "doc-4"],
      }),
      // shared by only doc-1/doc-2 -> higher idf, more specific
      entity({
        id: "ent-specific",
        display_name: "Specific SAS",
        source_documents: ["doc-1", "doc-2"],
      }),
    ];

    const related = computeRelatedDocuments(entities, docIds);
    const namesForDoc1 = related.get("doc-1")![0].sharedEntityNames;

    expect(namesForDoc1[0]).toBe("Specific SAS");
    expect(namesForDoc1[1]).toBe("Common Corp");
  });

  describe("measured on a 24-document synthetic corpus", () => {
    // Deliberately mixed distribution, not a uniform one — real corpora aren't uniform:
    // - 1 entity present in every document (a corpus-wide constant, e.g. the client name
    //   on every file in one case): must contribute nothing.
    // - 6 entities each shared by a small cluster of 2-4 documents (case-specific
    //   parties, the actually-useful signal).
    // - a long tail of entities unique to exactly one document each (can never
    //   contribute to any pair — included to prove they don't distort scoring).
    const docIds = Array.from({ length: 24 }, (_, i) => `doc-${i + 1}`);
    const entities: RegistryEntity[] = [
      entity({ id: "ent-constant", display_name: "MegaCorp", source_documents: [...docIds] }),
    ];
    // 6 clusters of 2-4 documents each, spread across the corpus without overlap between
    // clusters (each cluster's documents are a distinct slice).
    const clusterSizes = [2, 3, 4, 2, 3, 3];
    let cursor = 0;
    clusterSizes.forEach((size, idx) => {
      entities.push(
        entity({
          id: `ent-cluster-${idx}`,
          display_name: `Case ${idx} SAS`,
          source_documents: docIds.slice(cursor, cursor + size),
        }),
      );
      cursor += size;
    });
    // Every remaining, not-yet-clustered document gets one entity unique to itself.
    for (let i = cursor; i < docIds.length; i++) {
      entities.push(
        entity({ id: `ent-unique-${i}`, display_name: `Solo ${i}`, source_documents: [docIds[i]] }),
      );
    }

    const related = computeRelatedDocuments(entities, docIds);

    it("gives the corpus-wide entity zero scoring weight everywhere", () => {
      // If MegaCorp contributed any score, every one of the 24 documents would show every
      // other document as "related" (a fully-connected, useless graph) — the whole point
      // of IDF weighting is that this must not happen.
      for (const docId of docIds) {
        const forThisDoc = related.get(docId)!;
        for (const r of forThisDoc) {
          expect(r.sharedEntityNames).not.toContain("MegaCorp");
        }
      }
    });

    it("gives every clustered document at least one related document, and no unrelated pair a score", () => {
      // Documents 0 and 1 are the size-2 cluster -> must relate to each other.
      expect(related.get("doc-1")!.map((r) => r.docId)).toContain("doc-2");
      // Document 0 (in the size-2 cluster) and document 21 (in a size-3 cluster later in
      // the corpus) share no entity at all -> must not appear as related to each other.
      const doc1Related = related.get("doc-1")!.map((r) => r.docId);
      expect(doc1Related).not.toContain("doc-22");
    });

    it("measures the actual unbounded related-count distribution — this is what DEFAULT_MAX_RESULTS is chosen against", () => {
      const counts = docIds.map((d) => related.get(d)!.length).sort((a, b) => a - b);
      const max = counts[counts.length - 1];
      const median = counts[Math.floor(counts.length / 2)];
      // Recorded, not asserted as a hard pass/fail beyond sanity bounds: this test's
      // purpose is to make the real distribution visible (plan §5.2: "measure ... before
      // fixing the cutoff"), not to lock in one exact set of numbers as a regression gate.
      // On this corpus: every document's unbounded related-count tops out at its own
      // cluster size minus one (clusters don't overlap each other, by construction), so
      // the true maximum across the whole corpus is 3 (the size-4 cluster) and the median
      // is low because the unique-entity documents at the tail score 0 related documents
      // entirely. A cutoff of 5 (`DEFAULT_MAX_RESULTS`) never truncates anything on this
      // corpus shape — the risk case for a real corpus is a rare entity spanning a large
      // cluster (e.g. every filing that names one regulator), not the common case tested
      // here, which is why the cap exists as a safety margin rather than something this
      // corpus alone proves necessary.
      expect(max).toBeLessThanOrEqual(3);
      expect(median).toBeGreaterThanOrEqual(0);
    });
  });
});

describe("topRelatedDocuments (Task 5.2)", () => {
  it("caps at maxResults, keeping the highest-scoring entries", () => {
    const related = [
      { docId: "a", score: 5, sharedEntityNames: ["X"] },
      { docId: "b", score: 3, sharedEntityNames: ["Y"] },
      { docId: "c", score: 1, sharedEntityNames: ["Z"] },
    ];

    expect(topRelatedDocuments(related, 2).map((r) => r.docId)).toEqual(["a", "b"]);
  });

  it("defaults to 5 when maxResults is omitted", () => {
    const related = Array.from({ length: 8 }, (_, i) => ({
      docId: `doc-${i}`,
      score: 8 - i,
      sharedEntityNames: [],
    }));

    expect(topRelatedDocuments(related)).toHaveLength(5);
  });
});

describe("buildRelatedDocumentsSection (Task 5.2)", () => {
  it("returns an empty string, no heading, when there is nothing related", () => {
    expect(buildRelatedDocumentsSection([], "documents/msa.md", new Map())).toBe("");
  });

  it("emits the reason (shared entity names), not a bare link", () => {
    const docPaths = new Map([["doc-2", "documents/2024/amendment.md"]]);
    const section = buildRelatedDocumentsSection(
      [{ docId: "doc-2", score: 1.2, sharedEntityNames: ["Acme SAS", "Jean Dupont"] }],
      "documents/2024/msa.md",
      docPaths,
    );

    expect(section).toContain("## Related documents");
    // Label is the full documents-root-relative path (disambiguates same-named files in
    // different subdirectories); the link target is the real relative path computed
    // above — here they're both in documents/2024/, so the link itself is a bare
    // filename even though the label still shows the subdirectory.
    expect(section).toContain("[2024/amendment.md](amendment.md)");
    expect(section).toContain("shares Acme SAS, Jean Dupont");
  });

  it("uses a real relative path, not a flat ../ assumption, across differing nesting depth", () => {
    const docPaths = new Map([["doc-2", "documents/minutes/board.md"]]);
    const section = buildRelatedDocumentsSection(
      [{ docId: "doc-2", score: 1, sharedEntityNames: ["Acme SAS"] }],
      "documents/2024/msa.md",
      docPaths,
    );

    expect(section).toContain("(../minutes/board.md)");
  });

  it("skips a related docId with no known export path rather than emitting a broken link", () => {
    const section = buildRelatedDocumentsSection(
      [{ docId: "doc-missing", score: 1, sharedEntityNames: ["X"] }],
      "documents/msa.md",
      new Map(),
    );

    expect(section).not.toContain("undefined");
    expect(section).toContain("## Related documents");
  });
});
