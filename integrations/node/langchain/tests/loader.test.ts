import { mkdtemp, writeFile, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { HaciendaApiError } from "@hacienda-engine/sdk";
import { HaciendaLoader, HaciendaLoaderError } from "../src/loader.js";

/**
 * Duck-typed stand-in for `HaciendaClient` — `HaciendaLoader` only calls
 * `client.documents.processDocuments(body)` and reads plain fields off the result,
 * so a minimal fake is enough to exercise it without a live server.
 */
function fakeClient(opts: { response?: unknown; error?: Error } = {}) {
  const calls: unknown[] = [];
  return {
    documents: {
      processDocuments: vi.fn(async (body: unknown) => {
        calls.push(body);
        if (opts.error) throw opts.error;
        return opts.response;
      }),
    },
    calls,
  };
}

function entity(overrides: Partial<Record<string, unknown>> = {}) {
  return {
    category: "EMAIL",
    start: 0,
    end: 10,
    confidence: 0.95,
    source: "regex",
    ...overrides,
  };
}

function result(content: string, overrides: Partial<Record<string, unknown>> = {}) {
  return { content, entities: [], ...overrides };
}

function response(documents: unknown[], overrides: Partial<Record<string, unknown>> = {}) {
  return { documents, processing_time_ms: 12, audit_chain_tip: "blake3:abc123", ...overrides };
}

let dir: string;

beforeEach(async () => {
  dir = await mkdtemp(join(tmpdir(), "hacienda-loader-"));
});

afterEach(async () => {
  await rm(dir, { recursive: true, force: true });
  vi.unstubAllEnvs();
});

describe("HaciendaLoader", () => {
  it("requires filePath or data", () => {
    expect(() => new HaciendaLoader({ client: fakeClient() as never })).toThrow(
      /filePath.*or.*data/i,
    );
  });

  it("rejects both filePath and data", async () => {
    const path = join(dir, "a.txt");
    await writeFile(path, "hi");
    expect(
      () =>
        new HaciendaLoader({
          filePath: path,
          data: new Uint8Array([1]),
          client: fakeClient() as never,
        }),
    ).toThrow(/both/i);
  });

  it("requires mimeType with data", () => {
    expect(
      () => new HaciendaLoader({ data: new Uint8Array([1]), client: fakeClient() as never }),
    ).toThrow(/mimeType/i);
  });

  it("requires an API key when no client is given", () => {
    vi.stubEnv("HACIENDA_API_KEY", "");
    expect(
      () => new HaciendaLoader({ data: new Uint8Array([1]), mimeType: "text/plain" }),
    ).toThrow(/API key/i);
  });

  it("rejects documentId for a list of paths", async () => {
    const a = join(dir, "a.txt");
    const b = join(dir, "b.txt");
    await writeFile(a, "a");
    await writeFile(b, "b");
    expect(
      () =>
        new HaciendaLoader({
          filePath: [a, b],
          documentId: "00000000-0000-0000-0000-000000000000",
          client: fakeClient() as never,
        }),
    ).toThrow(/single-file/i);
  });

  it("loads a single file with redacted content and PII metadata", async () => {
    const path = join(dir, "note.txt");
    await writeFile(path, "Contact [EMAIL_REDACTED] for details.");

    const client = fakeClient({
      response: response([
        result("Contact [EMAIL_REDACTED] for details.", {
          entities: [entity({ category: "EMAIL", start: 8, end: 25 })],
        }),
      ]),
    });

    const loader = new HaciendaLoader({ filePath: path, client: client as never });
    const docs = await loader.load();

    expect(docs).toHaveLength(1);
    expect(docs[0].pageContent).toBe("Contact [EMAIL_REDACTED] for details.");
    expect(docs[0].metadata.source).toBe(path);
    expect(docs[0].metadata.pii_entities_found).toBe(1);
    expect(docs[0].metadata.pii_categories).toEqual(["EMAIL"]);
    expect(docs[0].metadata.audit_chain_tip).toBe("blake3:abc123");

    const sent = client.calls[0] as { documents: Array<{ mime_type: string }> };
    expect(sent.documents[0].mime_type).toBe("text/plain");
  });

  it("loads from bytes", async () => {
    const client = fakeClient({ response: response([result("hello redacted")]) });
    const loader = new HaciendaLoader({
      data: new TextEncoder().encode("hello world"),
      mimeType: "text/plain",
      filename: "mem.txt",
      client: client as never,
    });
    const docs = await loader.load();

    expect(docs[0].metadata.source).toBe("bytes://text/plain");
    expect(docs[0].metadata.pii_entities_found).toBe(0);
  });

  it("batches a directory into one request", async () => {
    await writeFile(join(dir, "a.txt"), "a");
    await writeFile(join(dir, "b.txt"), "b");

    const client = fakeClient({
      response: response([result("a redacted"), result("b redacted")]),
    });
    const loader = new HaciendaLoader({ filePath: dir, client: client as never });
    const docs = await loader.load();

    expect(docs).toHaveLength(2);
    expect(client.calls).toHaveLength(1);
  });

  it("rejects documentId for a directory once load() resolves it as a batch", async () => {
    await writeFile(join(dir, "a.txt"), "a");
    await writeFile(join(dir, "b.txt"), "b");

    // Unlike an explicit list of paths (rejected in the constructor, see above), a
    // directory's batch-ness is only known after load() resolves it via fastGlob —
    // this is the separate runtime check in load() itself.
    const loader = new HaciendaLoader({
      filePath: dir,
      documentId: "00000000-0000-0000-0000-000000000000",
      client: fakeClient() as never,
    });
    await expect(loader.load()).rejects.toThrow(/directory/i);
  });

  it("wraps API errors", async () => {
    const path = join(dir, "x.txt");
    await writeFile(path, "x");
    const client = fakeClient({
      error: new HaciendaApiError(403, "forbidden", "no documents:process"),
    });
    const loader = new HaciendaLoader({ filePath: path, client: client as never });
    await expect(loader.load()).rejects.toThrow(HaciendaLoaderError);
  });

  it("raises when a MIME type cannot be inferred", async () => {
    const path = join(dir, "mystery.xyz123");
    await writeFile(path, "?");
    const loader = new HaciendaLoader({ filePath: path, client: fakeClient() as never });
    await expect(loader.load()).rejects.toThrow(/MIME type/i);
  });
});
