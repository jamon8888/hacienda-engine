import { readFile, stat } from "node:fs/promises";
import { BaseDocumentLoader } from "@langchain/core/document_loaders/base";
import { Document } from "@langchain/core/documents";
import { HaciendaClient, type HaciendaApiError } from "@hacienda-engine/sdk";
import fastGlob from "fast-glob";
import mimeTypes from "mime-types";

const DEFAULT_GLOB = "**/*";

/** Mirrors `hacienda_sdk._generated.models.document_input.DocumentInput`'s wire shape. */
interface DocumentInput {
  content_base64: string;
  mime_type: string;
  filename?: string | null;
  document_id?: string | null;
}

export class HaciendaLoaderError extends Error {
  constructor(message: string, options?: { cause?: unknown }) {
    super(message, options);
    this.name = "HaciendaLoaderError";
  }
}

export interface HaciendaLoaderOptions {
  /** File path, list of file paths, or directory path to load. */
  filePath?: string | string[];
  /** Raw bytes to extract from. Mutually exclusive with `filePath`. */
  data?: Uint8Array;
  /** MIME type override. Required when using `data`; inferred from the file
   * extension for `filePath` when omitted. */
  mimeType?: string;
  /** Display filename to send alongside `data`. Ignored for `filePath`. */
  filename?: string;
  /** Glob pattern for directory mode. Defaults to matching all files. */
  glob?: string;
  /** Enables server-side document versioning. Single-file/`data` loads only —
   * every document in a batch would otherwise collide on the same id. */
  documentId?: string;
  /** A pre-built client. Takes precedence over `baseUrl`/`apiKey`; use this to
   * share one client (and its connection pool) across multiple loaders. */
  client?: HaciendaClient;
  /** Falls back to the `HACIENDA_BASE_URL` env var, then `http://127.0.0.1:8080`. */
  baseUrl?: string;
  /** Falls back to the `HACIENDA_API_KEY` env var. */
  apiKey?: string;
}

/**
 * Load documents through hacienda-engine's redacting, audited extraction pipeline.
 *
 * Unlike a loader that extracts raw text, every `Document` this yields has already
 * had PII removed per the server's configured redaction policy (`POST /v1/documents`
 * — see `hacienda-api/src/handlers/documents.rs`). Detected entities (category, span,
 * confidence — never raw text, since this endpoint never sets `include_text`) and the
 * batch's audit-chain tip land in `Document.metadata` alongside the usual fields.
 *
 * Multiple files, or a whole directory, are sent as **one batched request** — the
 * same reasoning `XbergLoader` (xberg's own LangChain.js loader) gives for batching
 * `extractBatch` calls server-side rather than looping client-side.
 */
export class HaciendaLoader extends BaseDocumentLoader {
  private readonly filePath?: string | string[];
  private readonly data?: Uint8Array;
  private readonly mimeType?: string;
  private readonly filename?: string;
  private readonly glob?: string;
  private readonly documentId?: string;
  private readonly client: HaciendaClient;

  constructor(options: HaciendaLoaderOptions) {
    super();
    const { filePath, data, mimeType, filename, glob, documentId, client, baseUrl, apiKey } =
      options;

    if (filePath === undefined && data === undefined) {
      throw new HaciendaLoaderError("Either 'filePath' or 'data' must be provided.");
    }
    if (filePath !== undefined && data !== undefined) {
      throw new HaciendaLoaderError(
        "Cannot specify both 'filePath' and 'data'. Use one or the other.",
      );
    }
    if (data !== undefined && mimeType === undefined) {
      throw new HaciendaLoaderError("'mimeType' is required when using 'data'.");
    }
    // A `filePath` directory only reveals whether it's a batch once resolved
    // (fastGlob, in `resolvePaths`), so that half of this check lives in `load()`;
    // this catches the always-known case (an explicit list) up front.
    if (documentId !== undefined && Array.isArray(filePath)) {
      throw new HaciendaLoaderError(
        "'documentId' is only valid for a single-file or bytes load, not a list of " +
          "paths — every document in a batch would otherwise collide on the same id.",
      );
    }

    this.filePath = filePath;
    this.data = data;
    this.mimeType = mimeType;
    this.filename = filename;
    this.glob = glob;
    this.documentId = documentId;

    if (client !== undefined) {
      this.client = client;
    } else {
      const resolvedBaseUrl =
        baseUrl ?? process.env.HACIENDA_BASE_URL ?? "http://127.0.0.1:8080";
      const resolvedApiKey = apiKey ?? process.env.HACIENDA_API_KEY;
      if (!resolvedApiKey) {
        throw new HaciendaLoaderError(
          "No API key: pass apiKey, set HACIENDA_API_KEY, or pass an existing client.",
        );
      }
      this.client = new HaciendaClient({ baseUrl: resolvedBaseUrl, apiKey: resolvedApiKey });
    }
  }

  async load(): Promise<Document[]> {
    const { documents, sources, batch } = await this.buildRequest();
    if (documents.length === 0) return [];

    if (this.documentId !== undefined && batch) {
      throw new HaciendaLoaderError(
        "'documentId' is only valid for a single-file or bytes load, not a directory " +
          "— every document in a batch would otherwise collide on the same id.",
      );
    }

    let response;
    try {
      response = await this.client.documents.processDocuments({ documents });
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error);
      throw new HaciendaLoaderError(`hacienda-engine request failed: ${message}`, {
        cause: error as HaciendaApiError,
      });
    }

    const auditChainTip = response.audit_chain_tip ?? undefined;
    return response.documents.map((result, index) => {
      const source = sources[index] ?? sources.at(-1) ?? "";
      const categories = [...new Set(result.entities.map((entity) => entity.category))].sort();

      const metadata: Record<string, unknown> = {
        source,
        pii_entities_found: result.entities.length,
        pii_categories: categories,
        pii_entities: result.entities.map((entity) => ({
          category: entity.category,
          start: entity.start,
          end: entity.end,
          confidence: entity.confidence,
          source: entity.source,
        })),
        processing_time_ms: response.processing_time_ms,
      };
      if (auditChainTip !== undefined) metadata.audit_chain_tip = auditChainTip;
      if (result.document_id != null) metadata.document_id = result.document_id;
      if (result.version_sequence != null) metadata.version_sequence = result.version_sequence;

      return new Document({ pageContent: result.content, metadata });
    });
  }

  private async buildRequest(): Promise<{
    documents: DocumentInput[];
    sources: string[];
    batch: boolean;
  }> {
    if (this.data !== undefined) {
      const source = `bytes://${this.mimeType}`;
      const input: DocumentInput = {
        content_base64: Buffer.from(this.data).toString("base64"),
        mime_type: this.mimeType as string,
        filename: this.filename ?? null,
        document_id: this.documentId ?? null,
      };
      return { documents: [input], sources: [source], batch: false };
    }

    const { paths, batch } = await this.resolvePaths();
    const documents: DocumentInput[] = [];
    for (const path of paths) {
      // `mimeTypes.lookup` returns `false` (not `undefined`) when it can't determine a
      // type, so `??` alone won't normalize a miss — check both explicitly.
      const looked = this.mimeType ?? mimeTypes.lookup(path);
      if (looked === undefined || looked === false) {
        throw new HaciendaLoaderError(
          `Could not infer a MIME type for '${path}'. Pass mimeType explicitly.`,
        );
      }
      const mimeType = looked;
      const content = await readFile(path);
      documents.push({
        content_base64: content.toString("base64"),
        mime_type: mimeType,
        filename: basename(path),
        document_id: batch ? undefined : (this.documentId ?? null),
      });
    }
    return { documents, sources: paths, batch };
  }

  private async resolvePaths(): Promise<{ paths: string[]; batch: boolean }> {
    const filePath = this.filePath;
    if (Array.isArray(filePath)) {
      return { paths: filePath, batch: true };
    }
    if (typeof filePath === "string") {
      if (await isDirectory(filePath)) {
        const pattern = this.glob ?? DEFAULT_GLOB;
        const matches = await fastGlob(pattern, { cwd: filePath, onlyFiles: true, absolute: true });
        return { paths: matches.sort(), batch: true };
      }
      return { paths: [filePath], batch: false };
    }
    return { paths: [], batch: false };
  }
}

function basename(path: string): string {
  const parts = path.split(/[/\\]/);
  return parts.at(-1) ?? path;
}

async function isDirectory(path: string): Promise<boolean> {
  try {
    const stats = await stat(path);
    return stats.isDirectory();
  } catch {
    return false;
  }
}
