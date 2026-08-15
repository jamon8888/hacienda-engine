import { BaseRetriever, type BaseRetrieverInput } from "@langchain/core/retrievers";
import type { CallbackManagerForRetrieverRun } from "@langchain/core/callbacks/manager";
import { Document } from "@langchain/core/documents";
import { HaciendaClient } from "@hacienda-engine/sdk";
import { HaciendaLoaderError } from "./loader.js";

/**
 * hacienda-engine RAG collection integration for LangChain.js: ingest + retrieve.
 *
 * Deliberately does not generate embeddings client-side — see this module's docstrings
 * and `../../LIMITATIONS.md` for what that means in practice (in short: ingestion
 * without a supplied `embeddings` array only works against a collection provisioned
 * with `embeddingDim: 0`, and the default retrieval mode, `"fulltext"`, only works
 * against a full-text-capable `RagStore` backend — the in-memory backend does not
 * support it. Pass `mode: "vector"` plus a `queryVector` through `HaciendaRetriever`
 * to use the in-memory backend, or any backend without full-text support).
 */

interface ClientOptions {
  client?: HaciendaClient;
  baseUrl?: string;
  apiKey?: string;
}

function resolveClient(options: ClientOptions): { client: HaciendaClient; ownsClient: boolean } {
  if (options.client !== undefined) {
    return { client: options.client, ownsClient: false };
  }
  const resolvedBaseUrl =
    options.baseUrl ?? process.env.HACIENDA_BASE_URL ?? "http://127.0.0.1:8080";
  const resolvedApiKey = options.apiKey ?? process.env.HACIENDA_API_KEY;
  if (!resolvedApiKey) {
    throw new HaciendaLoaderError(
      "No API key: pass apiKey, set HACIENDA_API_KEY, or pass an existing client.",
    );
  }
  return {
    client: new HaciendaClient({ baseUrl: resolvedBaseUrl, apiKey: resolvedApiKey }),
    ownsClient: true,
  };
}

export interface PushDocumentsOptions extends ClientOptions {
  /**
   * One embedding vector per document (same order), if you have your own. Sent as a
   * single whole-document chunk with that embedding attached. Omit to use naive
   * server-side full-text chunking instead — which only succeeds against a collection
   * provisioned with `embeddingDim: 0` (verified live: an omitted-chunks upsert into an
   * `embeddingDim > 0` collection fails with "embedding dimension mismatch: expected
   * N, got 0"). `pushDocuments` still never computes an embedding itself, it only
   * forwards one you already have (e.g. from a LangChain `Embeddings` model).
   */
  embeddings?: number[][];
}

/**
 * Ingest `Document`s into a hacienda RAG collection
 * (`POST /v1/rag/collections/{name}/documents`).
 *
 * The collection must already exist (`POST /v1/rag/collections` — provisioning one,
 * choosing its `embeddingDim`/`distanceMetric`/`indexMethod`, is a deployment
 * decision, not something this function makes for you).
 *
 * No PII redaction happens here — `RagStore::upsertDocument` is a pure storage write
 * (`crates/hacienda-rag/src/store.rs`). Feed this `HaciendaLoader.load()`'s output
 * (already redacted), not arbitrary raw text, if that matters to you.
 *
 * @returns The server-assigned `document_id` for each input document, same order.
 */
export async function pushDocuments(
  collection: string,
  documents: Document[],
  options: PushDocumentsOptions = {},
): Promise<string[]> {
  const { embeddings } = options;
  if (embeddings !== undefined && embeddings.length !== documents.length) {
    throw new HaciendaLoaderError(
      `'embeddings' has ${embeddings.length} vectors but 'documents' has ` +
        `${documents.length} — they must be the same length, same order.`,
    );
  }

  // No close()/cleanup needed on the resolved client either way —
  // @hacienda-engine/sdk's HaciendaClient is fetch-based with no connection pool to
  // release (unlike hacienda-sdk's Python HaciendaClient, which owns an httpx client
  // and does need one — see push_documents' Python twin).
  const { client } = resolveClient(options);
  const documentIds: string[] = [];
  for (const [index, doc] of documents.entries()) {
    const record: Record<string, unknown> = { full_text: doc.pageContent };
    if (doc.metadata && Object.keys(doc.metadata).length > 0) {
      record.metadata = doc.metadata;
      if (typeof doc.metadata.source === "string") {
        record.source_uri = doc.metadata.source;
      }
    }

    const body =
      embeddings !== undefined
        ? {
            document: record,
            chunks: [{ ordinal: 0, content: doc.pageContent, embedding: embeddings[index] }],
          }
        : { document: record };

    let response;
    try {
      response = await client.rag.upsertDocument(collection, body);
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error);
      throw new HaciendaLoaderError(`hacienda-engine request failed: ${message}`, {
        cause: error,
      });
    }
    documentIds.push(response.document_id);
  }
  return documentIds;
}

/** Shape of one `RetrievedChunk` in `POST /v1/rag/collections/{name}/retrieve`'s response. */
interface RetrievedChunk {
  id?: string;
  document_id?: string;
  ordinal?: number;
  content?: string;
  score?: number;
  chunk_metadata?: Record<string, unknown> | null;
}

export interface HaciendaRetrieverFields extends BaseRetrieverInput, ClientOptions {
  /** Name of an existing RAG collection. */
  collection: string;
  /** @default 10 */
  topK?: number;
  /**
   * Retrieval mode — one of hacienda_rag::RetrieveMode's wire values (note: no
   * underscore — `"fulltext"`/`"lateinteraction"`, not `"full_text"`/
   * `"late_interaction"`): `"vector"` (default), `"fulltext"`, `"hybrid"`,
   * `"sparse"`, `"lateinteraction"`.
   *
   * Defaults to `"fulltext"` here — the pairing that needs no client-side embedding
   * generation, matching `pushDocuments`'s naive-chunking ingest. This **only works
   * against a full-text-capable `RagStore` backend** (e.g. the pgvector backend); the
   * in-memory backend used by a bare `hacienda serve` reports
   * `Capabilities { full_text: false, .. }` and the request fails server-side with
   * "retrieval mode unsupported by backend". Pass `mode: "vector"` with your own
   * `queryVector` (via `retrieveExtra`) to use the in-memory backend, or any backend
   * without full-text support. See `../../LIMITATIONS.md`.
   */
  mode?: string;
  /** Extra fields merged into the retrieve request body (e.g. `{ query_vector: [...] }`). */
  retrieveExtra?: Record<string, unknown>;
}

/** Retrieve from a hacienda RAG collection (`POST /v1/rag/collections/{name}/retrieve`). */
export class HaciendaRetriever extends BaseRetriever {
  lc_namespace = ["hacienda", "retrievers"];

  private readonly collection: string;
  private readonly topK: number;
  private readonly mode: string;
  private readonly retrieveExtra: Record<string, unknown>;
  private readonly clientOptions: ClientOptions;

  constructor(fields: HaciendaRetrieverFields) {
    super(fields);
    this.collection = fields.collection;
    this.topK = fields.topK ?? 10;
    this.mode = fields.mode ?? "fulltext";
    this.retrieveExtra = fields.retrieveExtra ?? {};
    this.clientOptions = { client: fields.client, baseUrl: fields.baseUrl, apiKey: fields.apiKey };
  }

  async _getRelevantDocuments(
    query: string,
    _runManager?: CallbackManagerForRetrieverRun,
  ): Promise<Document[]> {
    const { client } = resolveClient(this.clientOptions);
    const body: Record<string, unknown> = {
      mode: this.mode,
      query_text: query,
      top_k: this.topK,
      include_content: true,
      include_document: true,
      ...this.retrieveExtra,
    };

    let chunks: RetrievedChunk[];
    try {
      // `retrieve`'s TS type is `unknown` (its OpenAPI response schema is empty —
      // see `../../LIMITATIONS.md`'s "bug found and fixed" note about the Python SDK's
      // equivalent break; openapi-fetch itself still returns the real body here, just
      // weakly typed). Validate the one field this method reads (`chunks` must be an
      // array, or absent) before touching it, rather than casting straight to the
      // expected shape — a malformed backend response should surface as a
      // HaciendaLoaderError, not a native TypeError from `.map()` on a non-array.
      const raw = await client.rag.retrieve(this.collection, body);
      const rawChunks = (raw as { chunks?: unknown } | undefined)?.chunks;
      if (rawChunks !== undefined && !Array.isArray(rawChunks)) {
        throw new HaciendaLoaderError(
          `hacienda-engine returned a malformed retrieve response: 'chunks' was ${typeof rawChunks}, expected an array.`,
        );
      }
      chunks = (rawChunks as RetrievedChunk[] | undefined) ?? [];
    } catch (error) {
      if (error instanceof HaciendaLoaderError) {
        throw error;
      }
      const message = error instanceof Error ? error.message : String(error);
      throw new HaciendaLoaderError(`hacienda-engine request failed: ${message}`, {
        cause: error,
      });
    }

    return chunks.map((chunk) => {
      const metadata: Record<string, unknown> = {
        score: chunk.score,
        document_id: chunk.document_id,
        chunk_id: chunk.id,
        ordinal: chunk.ordinal,
        ...(chunk.chunk_metadata ?? {}),
      };
      return new Document({ pageContent: chunk.content ?? "", metadata });
    });
  }
}
