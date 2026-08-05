/**
 * High-level client for the hacienda-engine API.
 *
 * One namespace property per OpenAPI tag (`client.pii.scanText(...)`,
 * `client.documents.processDocuments(...)`, `client.auth.getWhoami()`, ...),
 * each a thin wrapper over a typed `openapi-fetch` client generated from
 * `src/_generated/api.d.ts` (types only — openapi-fetch needs no
 * per-operation client code, unlike the Python package's
 * openapi-python-client output). Every wrapper method routes through
 * {@link unwrap}, which throws {@link HaciendaApiError} on any non-2xx
 * response instead of returning the `{ data, error }` pair openapi-fetch
 * itself resolves with — see errors.ts's doc comment for why.
 */

import createClient, { type Client } from "openapi-fetch";
import type { components, paths } from "./_generated/api.js";
import { unwrap } from "./errors.js";

export type { HaciendaApiError } from "./errors.js";

/**
 * `target: "device"` (an embedded Cactus runtime, no HTTP underneath) is
 * Phase 15 of the platform-parity plan and not implemented yet. The literal
 * is already a union of one so that phase can extend it without changing
 * this signature.
 */
export type Target = "cloud";

const DEFAULT_RETRY_STATUSES = new Set([429, 502, 503, 504]);
const RETRY_BACKOFF_BASE_MS = 200;

export interface HaciendaClientOptions {
  baseUrl: string;
  apiKey: string;
  target?: Target;
  /** Maximum additional attempts after the first, on a retryable status. */
  maxRetries?: number;
  retryStatuses?: Set<number>;
  fetch?: typeof globalThis.fetch;
}

function retryingFetch(
  baseFetch: typeof globalThis.fetch,
  maxRetries: number,
  retryStatuses: Set<number>,
): typeof globalThis.fetch {
  return async (input, init) => {
    let attempt = 0;
    for (;;) {
      const response = await baseFetch(input, init);
      if (!retryStatuses.has(response.status) || attempt >= maxRetries) {
        return response;
      }
      await response.body?.cancel();
      await new Promise((resolve) =>
        setTimeout(resolve, RETRY_BACKOFF_BASE_MS * 2 ** attempt),
      );
      attempt += 1;
    }
  };
}

type Schemas = components["schemas"];

export class HaciendaClient {
  private readonly api: Client<paths>;

  readonly info: {
    getHealth(): Promise<Schemas["HealthResponse"]>;
    getInfo(): Promise<Schemas["InfoResponse"]>;
    getVersion(): Promise<Schemas["VersionResponse"]>;
  };

  readonly documents: {
    processDocuments(
      body: Schemas["ProcessDocumentsRequest"],
    ): Promise<Schemas["ProcessDocumentsResponse"]>;
    processDocumentsBackground(
      body: Schemas["ProcessDocumentsRequest"],
    ): Promise<Schemas["AsyncJobResponse"]>;
  };

  readonly pii: {
    getPiiConfig(): Promise<Schemas["PiiConfigResponse"]>;
    redactText(
      body: Schemas["RedactTextRequest"],
    ): Promise<Schemas["RedactTextResponse"]>;
    revealToken(
      body: Schemas["RevealTokenRequest"],
    ): Promise<Schemas["RevealTokenResponse"]>;
    scanText(
      body: Schemas["ScanTextRequest"],
    ): Promise<Schemas["ScanTextResponse"]>;
  };

  readonly jobs: {
    getJob(id: string): Promise<Schemas["JobResponse"]>;
    getJobResult(id: string): Promise<Schemas["JobResultResponse"]>;
    listJobs(query?: {
      status?: string;
      limit?: number;
      offset?: number;
    }): Promise<Schemas["JobListResponse"]>;
  };

  readonly audit: {
    getAudit(): Promise<Schemas["AuditResponse"]>;
    verifyAudit(): Promise<Schemas["AuditVerifyResponse"]>;
  };

  readonly review: {
    getReview(): Promise<Schemas["ReviewResponse"]>;
    decideReview(
      id: string,
      body: Schemas["ReviewDecideRequest"],
    ): Promise<Schemas["ReviewDecideResponse"]>;
  };

  readonly compliance: {
    getComplianceDpia(): Promise<Schemas["ComplianceDpiaResponse"]>;
    getComplianceReport(): Promise<Schemas["ComplianceReportResponse"]>;
  };

  readonly glossary: {
    getGlossary(): Promise<Schemas["GlossaryResponse"]>;
  };

  readonly auth: {
    getAuthConfig(): Promise<Schemas["AuthConfigResponse"]>;
    getWhoami(): Promise<Schemas["WhoamiResponse"]>;
    issueKey(
      body: Schemas["IssueKeyRequest"],
    ): Promise<Schemas["IssueKeyResponse"]>;
    revokeKey(id: string): Promise<void>;
  };

  readonly rag: {
    listCollections(query?: {
      limit?: number;
      offset?: number;
    }): Promise<Schemas["ListCollectionsResponse"]>;
    createCollection(body: unknown): Promise<unknown>;
    getCollection(name: string): Promise<unknown>;
    deleteCollection(name: string): Promise<void>;
    listDocuments(
      name: string,
      query?: { limit?: number; offset?: number },
    ): Promise<Schemas["ListDocumentsResponse"]>;
    upsertDocument(
      name: string,
      body: Schemas["UpsertDocumentRequest"],
    ): Promise<Schemas["UpsertDocumentResponse"]>;
    retrieve(name: string, body: unknown): Promise<unknown>;
    migrateEmbeddings(
      name: string,
      body: Schemas["MigrateEmbeddingsRequest"],
    ): Promise<Schemas["MigrateEmbeddingsResponse"]>;
    getMigrateStatus(
      name: string,
      jobId: string,
    ): Promise<Schemas["MigrateStatusResponse"]>;
  };

  readonly presets: {
    listPresets(): Promise<Schemas["PresetListResponse"]>;
    createPreset(
      body: Schemas["CreatePresetRequest"],
    ): Promise<Schemas["PresetResponse"]>;
    getPreset(id: string): Promise<Schemas["PresetResponse"]>;
    deletePreset(id: string): Promise<void>;
  };

  readonly versions: {
    listDocumentVersions(
      id: string,
    ): Promise<Schemas["DocumentVersionListResponse"]>;
    getDocument(id: string): Promise<Schemas["DocumentEnvelopeResponse"]>;
    // Synchronous by default (200, DocumentDiffResponse); over the server's
    // 2-second budget it returns 202 + a diff_job_id instead of blocking —
    // see `getDiffJob` to poll that fallback. Both are 2xx, so `unwrap`
    // returns whichever the server chose; callers branch on the shape.
    diffDocument(
      id: string,
      query: { from: number; to: number },
    ): Promise<
      Schemas["DocumentDiffResponse"] | Schemas["DiffJobAcceptedResponse"]
    >;
    getDiffJob(
      id: string,
      diffJobId: string,
    ): Promise<Schemas["DiffJobResultResponse"]>;
  };

  readonly uploads: {
    presignUpload(
      body: Schemas["PresignUploadRequest"],
    ): Promise<Schemas["PresignUploadResponse"]>;
    confirmUpload(
      body: Schemas["ConfirmUploadRequest"],
    ): Promise<Schemas["ConfirmUploadResponse"]>;
  };

  readonly usage: {
    getUsage(query?: {
      since?: string;
      until?: string;
    }): Promise<Schemas["UsageResponse"]>;
  };

  constructor(options: HaciendaClientOptions) {
    const target = options.target ?? "cloud";
    if (target !== "cloud") {
      throw new Error(
        `unsupported target ${JSON.stringify(target)} — only "cloud" is implemented today`,
      );
    }

    const maxRetries = options.maxRetries ?? 2;
    const retryStatuses = options.retryStatuses ?? DEFAULT_RETRY_STATUSES;
    const baseFetch = options.fetch ?? globalThis.fetch;

    this.api = createClient<paths>({
      baseUrl: options.baseUrl,
      headers: { Authorization: `Bearer ${options.apiKey}` },
      fetch: retryingFetch(baseFetch, maxRetries, retryStatuses),
    });

    const api = this.api;

    this.info = {
      getHealth: async () => unwrap(await api.GET("/health")),
      getInfo: async () => unwrap(await api.GET("/info")),
      getVersion: async () => unwrap(await api.GET("/version")),
    };

    this.documents = {
      processDocuments: async (body) =>
        unwrap(await api.POST("/v1/documents", { body })),
      processDocumentsBackground: async (body) =>
        unwrap(await api.POST("/v1/documents/async", { body })),
    };

    this.pii = {
      getPiiConfig: async () => unwrap(await api.GET("/v1/pii/config")),
      redactText: async (body) =>
        unwrap(await api.POST("/v1/pii/redact", { body })),
      revealToken: async (body) =>
        unwrap(await api.POST("/v1/pii/reveal", { body })),
      scanText: async (body) =>
        unwrap(await api.POST("/v1/pii/scan", { body })),
    };

    this.jobs = {
      getJob: async (id) =>
        unwrap(await api.GET("/v1/jobs/{id}", { params: { path: { id } } })),
      getJobResult: async (id) =>
        unwrap(
          await api.GET("/v1/jobs/{id}/result", { params: { path: { id } } }),
        ),
      listJobs: async (query) =>
        unwrap(await api.GET("/v1/jobs", { params: { query } })),
    };

    this.audit = {
      getAudit: async () => unwrap(await api.GET("/v1/audit")),
      verifyAudit: async () => unwrap(await api.GET("/v1/audit/verify")),
    };

    this.review = {
      getReview: async () => unwrap(await api.GET("/v1/review")),
      decideReview: async (id, body) =>
        unwrap(
          await api.POST("/v1/review/{id}/decide", {
            params: { path: { id } },
            body,
          }),
        ),
    };

    this.compliance = {
      getComplianceDpia: async () =>
        unwrap(await api.GET("/v1/compliance/dpia")),
      getComplianceReport: async () =>
        unwrap(await api.GET("/v1/compliance/report")),
    };

    this.glossary = {
      getGlossary: async () => unwrap(await api.GET("/v1/glossary")),
    };

    this.auth = {
      getAuthConfig: async () => unwrap(await api.GET("/v1/auth/config")),
      getWhoami: async () => unwrap(await api.GET("/v1/auth/whoami")),
      issueKey: async (body) =>
        unwrap(await api.POST("/v1/auth/keys", { body })),
      revokeKey: async (id) =>
        unwrap(
          await api.DELETE("/v1/auth/keys/{id}", { params: { path: { id } } }),
        ),
    };

    this.rag = {
      listCollections: async (query) =>
        unwrap(await api.GET("/v1/rag/collections", { params: { query } })),
      createCollection: async (body) =>
        unwrap(await api.POST("/v1/rag/collections", { body })),
      getCollection: async (name) =>
        unwrap(
          await api.GET("/v1/rag/collections/{name}", {
            params: { path: { name } },
          }),
        ),
      deleteCollection: async (name) =>
        unwrap(
          await api.DELETE("/v1/rag/collections/{name}", {
            params: { path: { name } },
          }),
        ),
      listDocuments: async (name, query) =>
        unwrap(
          await api.GET("/v1/rag/collections/{name}/documents", {
            params: { path: { name }, query },
          }),
        ),
      upsertDocument: async (name, body) =>
        unwrap(
          await api.POST("/v1/rag/collections/{name}/documents", {
            params: { path: { name } },
            body,
          }),
        ),
      retrieve: async (name, body) =>
        unwrap(
          await api.POST("/v1/rag/collections/{name}/retrieve", {
            params: { path: { name } },
            body,
          }),
        ),
      migrateEmbeddings: async (name, body) =>
        unwrap(
          await api.POST("/v1/rag/collections/{name}/migrate-embeddings", {
            params: { path: { name } },
            body,
          }),
        ),
      getMigrateStatus: async (name, jobId) =>
        unwrap(
          await api.GET(
            "/v1/rag/collections/{name}/migrate-embeddings/{job_id}",
            {
              params: { path: { name, job_id: jobId } },
            },
          ),
        ),
    };

    this.presets = {
      listPresets: async () => unwrap(await api.GET("/v1/presets")),
      createPreset: async (body) =>
        unwrap(await api.POST("/v1/presets", { body })),
      getPreset: async (id) =>
        unwrap(await api.GET("/v1/presets/{id}", { params: { path: { id } } })),
      deletePreset: async (id) =>
        unwrap(
          await api.DELETE("/v1/presets/{id}", { params: { path: { id } } }),
        ),
    };

    this.versions = {
      listDocumentVersions: async (id) =>
        unwrap(
          await api.GET("/v1/documents/{id}/versions", {
            params: { path: { id } },
          }),
        ),
      getDocument: async (id) =>
        unwrap(
          await api.GET("/v1/documents/{id}", { params: { path: { id } } }),
        ),
      diffDocument: async (id, query) =>
        unwrap(
          await api.GET("/v1/documents/{id}/diff", {
            params: { path: { id }, query },
          }),
        ),
      getDiffJob: async (id, diffJobId) =>
        unwrap(
          await api.GET("/v1/documents/{id}/diff/{diff_job_id}", {
            params: { path: { id, diff_job_id: diffJobId } },
          }),
        ),
    };

    this.uploads = {
      presignUpload: async (body) =>
        unwrap(await api.POST("/v1/uploads/presign", { body })),
      confirmUpload: async (body) =>
        unwrap(await api.POST("/v1/uploads/confirm", { body })),
    };

    this.usage = {
      getUsage: async (query) =>
        unwrap(await api.GET("/v1/usage", { params: { query } })),
    };
  }

  /**
   * `GET /v1/auth/whoami` — the calling key's own granted capabilities. The
   * capability probe a client should use to decide which methods will
   * actually work against the supplied key, since hacienda has one tier
   * (unlike xberg's `_resolve_tier`, which this replaces): see
   * `hacienda-api/src/handlers/auth.rs`'s `whoami` doc comment.
   */
  async whoami(): Promise<Schemas["WhoamiResponse"]> {
    return this.auth.getWhoami();
  }
}
