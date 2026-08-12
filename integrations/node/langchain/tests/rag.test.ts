import { describe, it, expect, vi } from "vitest";
import { Document } from "@langchain/core/documents";
import { HaciendaApiError, type HaciendaClient } from "@hacienda-engine/sdk";
import { HaciendaLoaderError } from "../src/loader.js";
import { HaciendaRetriever, pushDocuments } from "../src/rag.js";

type UpsertCall = [collection: string, body: { document: Record<string, unknown>; chunks?: unknown }];
type RetrieveCall = [collection: string, body: Record<string, unknown>];

/**
 * A minimal stand-in for `HaciendaClient` exposing only the `rag` namespace these
 * tests exercise. Cast through `unknown` (not `never`) at call sites — `never` would
 * silently accept a fake shaped nothing like the real client, `unknown` still forces
 * an explicit, visible cast.
 */
function fakeRagClient(opts: {
  upsertResponse?: unknown;
  retrieveResponse?: unknown;
  error?: Error;
} = {}) {
  const upsertCalls: UpsertCall[] = [];
  const retrieveCalls: RetrieveCall[] = [];
  return {
    rag: {
      upsertDocument: vi.fn(async (collection: string, body: unknown) => {
        upsertCalls.push([collection, body as UpsertCall[1]]);
        if (opts.error) throw opts.error;
        return opts.upsertResponse;
      }),
      retrieve: vi.fn(async (collection: string, body: unknown) => {
        retrieveCalls.push([collection, body as RetrieveCall[1]]);
        if (opts.error) throw opts.error;
        return opts.retrieveResponse;
      }),
    },
    upsertCalls,
    retrieveCalls,
  };
}

function asClient(fake: ReturnType<typeof fakeRagClient>): HaciendaClient {
  return fake as unknown as HaciendaClient;
}

describe("pushDocuments", () => {
  it("sends full_text and metadata without chunks by default", async () => {
    const client = fakeRagClient({ upsertResponse: { document_id: "doc-1" } });
    const docs = [
      new Document({ pageContent: "hello redacted", metadata: { source: "a.txt", pii_entities_found: 0 } }),
    ];

    const ids = await pushDocuments("my-collection", docs, { client: asClient(client) });

    expect(ids).toEqual(["doc-1"]);
    expect(client.upsertCalls).toHaveLength(1);
    const [collection, body] = client.upsertCalls[0];
    expect(collection).toBe("my-collection");
    expect(body.document.full_text).toBe("hello redacted");
    expect(body.document.source_uri).toBe("a.txt");
    expect((body.document.metadata as Record<string, unknown>).pii_entities_found).toBe(0);
    expect(body.chunks).toBeUndefined();
  });

  it("sends supplied embeddings as a single chunk", async () => {
    const client = fakeRagClient({ upsertResponse: { document_id: "doc-2" } });
    const docs = [new Document({ pageContent: "vectorizable text" })];

    await pushDocuments("coll", docs, { embeddings: [[0.1, 0.2, 0.3]], client: asClient(client) });

    expect(client.upsertCalls).toHaveLength(1);
    const [, body] = client.upsertCalls[0];
    expect(body.chunks).toEqual([{ ordinal: 0, content: "vectorizable text", embedding: [0.1, 0.2, 0.3] }]);
  });

  it("rejects mismatched embeddings length", async () => {
    const client = fakeRagClient();
    const docs = [new Document({ pageContent: "a" }), new Document({ pageContent: "b" })];

    await expect(
      pushDocuments("coll", docs, { embeddings: [[0.1]], client: asClient(client) }),
    ).rejects.toThrow(/same length/i);
  });

  it("batches multiple documents as separate calls", async () => {
    const client = fakeRagClient({ upsertResponse: { document_id: "doc-x" } });
    const docs = [new Document({ pageContent: "a" }), new Document({ pageContent: "b" })];

    const ids = await pushDocuments("coll", docs, { client: asClient(client) });

    expect(ids).toEqual(["doc-x", "doc-x"]);
    expect(client.upsertCalls).toHaveLength(2);
  });

  it("wraps API errors", async () => {
    const client = fakeRagClient({ error: new HaciendaApiError(403, "forbidden", "no rag:write") });
    await expect(
      pushDocuments("coll", [new Document({ pageContent: "x" })], { client: asClient(client) }),
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
    const retriever = new HaciendaRetriever({ collection: "my-collection", client: asClient(client) });

    const docs = await retriever.invoke("acme contract");

    expect(docs).toHaveLength(1);
    const [doc] = docs;
    expect(doc.pageContent).toBe("Contract text");
    expect(doc.metadata.score).toBe(0.87);
    expect(doc.metadata.document_id).toBe("doc-1");
    expect(doc.metadata.chunk_id).toBe("doc-1:0");
    expect(doc.metadata.heading).toBe("Intro");

    expect(client.retrieveCalls).toHaveLength(1);
    const [collection, body] = client.retrieveCalls[0];
    expect(collection).toBe("my-collection");
    expect(body.mode).toBe("fulltext");
    expect(body.query_text).toBe("acme contract");
    expect(body.top_k).toBe(10);
    expect(body.include_content).toBe(true);
  });

  it("returns no documents for an empty result", async () => {
    const client = fakeRagClient({ retrieveResponse: { mode: "vector", chunks: [], primary_latency_ms: 1 } });
    const retriever = new HaciendaRetriever({ collection: "coll", client: asClient(client) });

    const docs = await retriever.invoke("nothing matches");

    expect(docs).toEqual([]);
  });

  it("respects custom mode and topK", async () => {
    const client = fakeRagClient({ retrieveResponse: { mode: "vector", chunks: [], primary_latency_ms: 1 } });
    const retriever = new HaciendaRetriever({
      collection: "coll",
      mode: "vector",
      topK: 3,
      client: asClient(client),
    });

    await retriever.invoke("q");

    expect(client.retrieveCalls).toHaveLength(1);
    const [, body] = client.retrieveCalls[0];
    expect(body.mode).toBe("vector");
    expect(body.top_k).toBe(3);
  });

  it("merges retrieveExtra into the request body", async () => {
    const client = fakeRagClient({ retrieveResponse: { mode: "vector", chunks: [], primary_latency_ms: 1 } });
    const retriever = new HaciendaRetriever({
      collection: "coll",
      mode: "vector",
      client: asClient(client),
      retrieveExtra: { query_vector: [0.1, 0.2] },
    });

    await retriever.invoke("q");

    expect(client.retrieveCalls).toHaveLength(1);
    const [, body] = client.retrieveCalls[0];
    expect(body.query_vector).toEqual([0.1, 0.2]);
  });

  it("wraps API errors", async () => {
    const client = fakeRagClient({ error: new HaciendaApiError(400, "invalid_request", "bad query") });
    const retriever = new HaciendaRetriever({ collection: "coll", client: asClient(client) });

    await expect(retriever.invoke("q")).rejects.toThrow(HaciendaLoaderError);
  });

  it("wraps a malformed response (non-array chunks) as HaciendaLoaderError", async () => {
    const client = fakeRagClient({ retrieveResponse: { mode: "vector", chunks: "not-an-array" } });
    const retriever = new HaciendaRetriever({ collection: "coll", client: asClient(client) });

    await expect(retriever.invoke("q")).rejects.toThrow(HaciendaLoaderError);
    await expect(retriever.invoke("q")).rejects.toThrow(/malformed/i);
  });
});
