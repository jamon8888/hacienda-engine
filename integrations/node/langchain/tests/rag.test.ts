import { describe, it, expect, vi } from "vitest";
import { Document } from "@langchain/core/documents";
import { HaciendaApiError } from "@hacienda-engine/sdk";
import { HaciendaLoaderError } from "../src/loader.js";
import { HaciendaRetriever, pushDocuments } from "../src/rag.js";

function fakeRagClient(opts: {
  upsertResponse?: unknown;
  retrieveResponse?: unknown;
  error?: Error;
} = {}) {
  const upsertCalls: unknown[] = [];
  const retrieveCalls: unknown[] = [];
  return {
    rag: {
      upsertDocument: vi.fn(async (collection: string, body: unknown) => {
        upsertCalls.push([collection, body]);
        if (opts.error) throw opts.error;
        return opts.upsertResponse;
      }),
      retrieve: vi.fn(async (collection: string, body: unknown) => {
        retrieveCalls.push([collection, body]);
        if (opts.error) throw opts.error;
        return opts.retrieveResponse;
      }),
    },
    upsertCalls,
    retrieveCalls,
  };
}

describe("pushDocuments", () => {
  it("sends full_text and metadata without chunks by default", async () => {
    const client = fakeRagClient({ upsertResponse: { document_id: "doc-1" } });
    const docs = [
      new Document({ pageContent: "hello redacted", metadata: { source: "a.txt", pii_entities_found: 0 } }),
    ];

    const ids = await pushDocuments("my-collection", docs, { client: client as never });

    expect(ids).toEqual(["doc-1"]);
    const [collection, body] = client.upsertCalls[0] as [string, { document: Record<string, unknown>; chunks?: unknown }];
    expect(collection).toBe("my-collection");
    expect(body.document.full_text).toBe("hello redacted");
    expect(body.document.source_uri).toBe("a.txt");
    expect((body.document.metadata as Record<string, unknown>).pii_entities_found).toBe(0);
    expect(body.chunks).toBeUndefined();
  });

  it("sends supplied embeddings as a single chunk", async () => {
    const client = fakeRagClient({ upsertResponse: { document_id: "doc-2" } });
    const docs = [new Document({ pageContent: "vectorizable text" })];

    await pushDocuments("coll", docs, { embeddings: [[0.1, 0.2, 0.3]], client: client as never });

    const [, body] = client.upsertCalls[0] as [string, { chunks: unknown }];
    expect(body.chunks).toEqual([{ ordinal: 0, content: "vectorizable text", embedding: [0.1, 0.2, 0.3] }]);
  });

  it("rejects mismatched embeddings length", async () => {
    const client = fakeRagClient();
    const docs = [new Document({ pageContent: "a" }), new Document({ pageContent: "b" })];

    await expect(
      pushDocuments("coll", docs, { embeddings: [[0.1]], client: client as never }),
    ).rejects.toThrow(/same length/i);
  });

  it("batches multiple documents as separate calls", async () => {
    const client = fakeRagClient({ upsertResponse: { document_id: "doc-x" } });
    const docs = [new Document({ pageContent: "a" }), new Document({ pageContent: "b" })];

    const ids = await pushDocuments("coll", docs, { client: client as never });

    expect(ids).toEqual(["doc-x", "doc-x"]);
    expect(client.upsertCalls).toHaveLength(2);
  });

  it("wraps API errors", async () => {
    const client = fakeRagClient({ error: new HaciendaApiError(403, "forbidden", "no rag:write") });
    await expect(
      pushDocuments("coll", [new Document({ pageContent: "x" })], { client: client as never }),
    ).rejects.toThrow(HaciendaLoaderError);
  });
});

describe("HaciendaRetriever", () => {
  it("maps chunks to Documents", async () => {
    const client = fakeRagClient({
      retrieveResponse: {
        mode: "fulltext",
        chunks: [
          {
            id: "doc-1:0",
            document_id: "doc-1",
            ordinal: 0,
            content: "Contract text",
            score: 0.87,
            chunk_metadata: { heading: "Intro" },
          },
        ],
        primary_latency_ms: 3,
      },
    });
    const retriever = new HaciendaRetriever({ collection: "my-collection", client: client as never });

    const docs = await retriever.invoke("acme contract");

    expect(docs).toHaveLength(1);
    expect(docs[0].pageContent).toBe("Contract text");
    expect(docs[0].metadata.score).toBe(0.87);
    expect(docs[0].metadata.document_id).toBe("doc-1");
    expect(docs[0].metadata.chunk_id).toBe("doc-1:0");
    expect(docs[0].metadata.heading).toBe("Intro");

    const [collection, body] = client.retrieveCalls[0] as [string, Record<string, unknown>];
    expect(collection).toBe("my-collection");
    expect(body.mode).toBe("fulltext");
    expect(body.query_text).toBe("acme contract");
    expect(body.top_k).toBe(10);
    expect(body.include_content).toBe(true);
  });

  it("returns no documents for an empty result", async () => {
    const client = fakeRagClient({ retrieveResponse: { mode: "vector", chunks: [], primary_latency_ms: 1 } });
    const retriever = new HaciendaRetriever({ collection: "coll", client: client as never });

    const docs = await retriever.invoke("nothing matches");

    expect(docs).toEqual([]);
  });

  it("respects custom mode and topK", async () => {
    const client = fakeRagClient({ retrieveResponse: { mode: "vector", chunks: [], primary_latency_ms: 1 } });
    const retriever = new HaciendaRetriever({
      collection: "coll",
      mode: "vector",
      topK: 3,
      client: client as never,
    });

    await retriever.invoke("q");

    const [, body] = client.retrieveCalls[0] as [string, Record<string, unknown>];
    expect(body.mode).toBe("vector");
    expect(body.top_k).toBe(3);
  });

  it("merges retrieveExtra into the request body", async () => {
    const client = fakeRagClient({ retrieveResponse: { mode: "vector", chunks: [], primary_latency_ms: 1 } });
    const retriever = new HaciendaRetriever({
      collection: "coll",
      mode: "vector",
      client: client as never,
      retrieveExtra: { query_vector: [0.1, 0.2] },
    });

    await retriever.invoke("q");

    const [, body] = client.retrieveCalls[0] as [string, Record<string, unknown>];
    expect(body.query_vector).toEqual([0.1, 0.2]);
  });

  it("wraps API errors", async () => {
    const client = fakeRagClient({ error: new HaciendaApiError(400, "invalid_request", "bad query") });
    const retriever = new HaciendaRetriever({ collection: "coll", client: client as never });

    await expect(retriever.invoke("q")).rejects.toThrow(HaciendaLoaderError);
  });
});
