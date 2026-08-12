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
import { JobTimeoutError, unwrap } from "./errors.js";

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
const RETRY_JITTER_MS = 100;

// The API has no idempotency-key mechanism for POST, so a retried POST
// (e.g. /v1/documents, /v1/uploads/confirm) can create a duplicate resource
// if the origin processed the first attempt before a gateway returned
// 502/503/504. Only replay methods that are safe to repeat by HTTP
// semantics — never POST. DELETE is included because the SDK's own delete
// endpoints (revokeKey, deleteCollection, deletePreset) are idempotent by
// design: repeating them against an already-deleted resource is a no-op.
const IDEMPOTENT_METHODS = new Set(["GET", "HEAD", "OPTIONS", "DELETE"]);

// Terminal states for `waitForJob(s)`/`extractAndWait` — see `JobStatus` in
// `hacienda-core/src/jobs/types.rs`. A `failed` job is still terminal: it is
// returned to the caller, not thrown, since the job itself finished running.
const TERMINAL_JOB_STATUSES = new Set(["succeeded", "failed"]);
const DEFAULT_POLL_INTERVAL_MS = 1000;
const DEFAULT_JOB_TIMEOUT_MS = 300_000;

function sleep(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

export interface WaitOptions {
  /** Milliseconds between polls of the job endpoint. Default 1000. */
  pollIntervalMs?: number;
  /** Total milliseconds to wait before throwing `JobTimeoutError`. Default 300000. */
  timeoutMs?: number;
}

export interface HaciendaClientOptions {
  baseUrl: string;
  apiKey: string;
  target?: Target;
  /** Maximum additional attempts after the first, on a retryable status. */
  maxRetries?: number;
  retryStatuses?: Set<number>;
  fetch?: typeof globalThis.fetch;
}

function retryAfterMs(response: Response): number | undefined {
  const header = response.headers.get("retry-after");
  if (header === null) return undefined;
  const seconds = Number(header);
  return Number.isFinite(seconds) ? seconds * 1000 : undefined;
}

function retryingFetch(
  baseFetch: typeof globalThis.fetch,
  maxRetries: number,
  retryStatuses: Set<number>,
): typeof globalThis.fetch {
  return async (input, init) => {
    const method = (init?.method ?? "GET").toUpperCase();
    const replayable = IDEMPOTENT_METHODS.has(method);
    let attempt = 0;
    for (;;) {
      const response = await baseFetch(input, init);
      if (
        !replayable ||
        !retryStatuses.has(response.status) ||
        attempt >= maxRetries
      ) {
        return response;
      }
      const delay =
        retryAfterMs(response) ??
        RETRY_BACKOFF_BASE_MS * 2 ** attempt + Math.random() * RETRY_JITTER_MS;
      await response.body?.cancel().catch(() => undefined);
      await new Promise((resolve) => setTimeout(resolve, delay));
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
    // Backed by the richer `/v1/audit/verify` (broken-chain-as-200, names the
    // offending entry/seal) — see hacienda-api/src/routes.rs's `/v1/audit*` comment.
    verifyAudit(): Promise<Schemas["VerifyResponse"]>;
    /**
     * `GET /v1/audit/entries` — one page of this node's history, oldest
     * first. Page until you receive an **empty page**, not until
     * `next_cursor` is absent — see
     * `hacienda-api/src/handlers/audit.rs`'s module doc.
     */
    getAuditEntries(query?: {
      limit?: number;
      cursor?: string;
    }): Promise<Schemas["NodeAuditPage"]>;
    /** `GET /v1/audit/seals` — every segment seal this node holds, oldest first. */
    getAuditSeals(): Promise<Schemas["NodeSealsResponse"]>;
    /**
     * `GET /v1/audit/export` — the whole history as bytes. `format` is
     * `json` (default) or `jsonl` (both verifiable offline via chain_hash
     * recomputation) or `csv` (a tabular extract, not verifiable). Requires
     * `audit:export`.
     *
     * Typed as `unknown`: the response body's shape (and content-type)
     * depends on `format` — `json`/`jsonl` are JSON, `csv` is a plain-text
     * table — which openapi-fetch's generated types cannot express as one
     * schema. Callers requesting `csv` may need to read the raw response
     * text rather than relying on this being parsed JSON.
     */
    exportAudit(query?: { format?: "json" | "jsonl" | "csv" }): Promise<unknown>;
    /** `GET /v1/audit/tip` — the current head of this node's chain. */
    getAuditTip(): Promise<Schemas["AuditTipResponse"]>;
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
    /**
     * `POST /v1/rag/collections/{name}/documents/{id}/reindex` — re-derives
     * the document's chunks from its stored `full_text` without resubmitting
     * content. Requires the document to have been upserted with
     * `external_id` set — see the route's own doc comment
     * (`hacienda-api/src/handlers/rag.rs`) for why.
     */
    reindexDocument(
      name: string,
      id: string,
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
    /**
     * `POST /v1/rag/collections/{name}/answer` — streaming RAG answer
     * synthesis over already-retrieved, PII-redaction-gated chunks.
     *
     * Typed as `unknown`: the response is `text/event-stream` (SSE
     * `citation`/`token`/`usage`/`done`/`error` events), which openapi-fetch's
     * typed JSON layer does not model. Mirrors `sdks/python`'s existing
     * `rag.answer()`, which has the same limitation — neither client parses
     * the SSE frames yet; both hand back the raw response body. A caller
     * that needs real token-by-token streaming should call the endpoint
     * directly with `fetch` and parse `text/event-stream` itself until a
     * typed SSE reader ships here.
     */
    answer(name: string, body: Schemas["AnswerRequest"]): Promise<unknown>;
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
      getAuditEntries: async (query) =>
        unwrap(await api.GET("/v1/audit/entries", { params: { query } })),
      getAuditSeals: async () => unwrap(await api.GET("/v1/audit/seals")),
      exportAudit: async (query) =>
        unwrap(await api.GET("/v1/audit/export", { params: { query } })),
      getAuditTip: async () => unwrap(await api.GET("/v1/audit/tip")),
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
      reindexDocument: async (name, id) =>
        unwrap(
          await api.POST(
            "/v1/rag/collections/{name}/documents/{id}/reindex",
            {
              params: { path: { name, id } },
            },
          ),
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
      answer: async (name, body) =>
        unwrap(
          await api.POST("/v1/rag/collections/{name}/answer", {
            params: { path: { name } },
            body,
          }),
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

  /**
   * Poll `GET /v1/jobs/{id}/result` until `jobId` reaches a terminal state.
   *
   * Mirrors xberg-sdks' `wait_for_job`. Resolves with the terminal
   * `JobResultResponse` — a `failed` job resolves, it does not reject, since
   * the job itself finished running; its `error` field carries the reason.
   *
   * Throws {@link JobTimeoutError} if `timeoutMs` elapses first; the job may
   * still be running server-side when that happens.
   */
  async waitForJob(
    jobId: string,
    options: WaitOptions = {},
  ): Promise<Schemas["JobResultResponse"]> {
    const pollIntervalMs = options.pollIntervalMs ?? DEFAULT_POLL_INTERVAL_MS;
    const timeoutMs = options.timeoutMs ?? DEFAULT_JOB_TIMEOUT_MS;
    const deadline = Date.now() + timeoutMs;
    for (;;) {
      const result = await this.jobs.getJobResult(jobId);
      if (TERMINAL_JOB_STATUSES.has(result.status)) {
        return result;
      }
      if (Date.now() >= deadline) {
        throw new JobTimeoutError(jobId, timeoutMs);
      }
      await sleep(pollIntervalMs);
    }
  }

  /**
   * `waitForJob` for several jobs at once, sharing one deadline.
   *
   * Resolves with a `Map` keyed by job id, same jobs as `jobIds`. Throws
   * {@link JobTimeoutError} naming one still-pending job if `timeoutMs`
   * elapses before every job reaches a terminal state.
   */
  async waitForJobs(
    jobIds: string[],
    options: WaitOptions = {},
  ): Promise<Map<string, Schemas["JobResultResponse"]>> {
    const pollIntervalMs = options.pollIntervalMs ?? DEFAULT_POLL_INTERVAL_MS;
    const timeoutMs = options.timeoutMs ?? DEFAULT_JOB_TIMEOUT_MS;
    const deadline = Date.now() + timeoutMs;
    const results = new Map<string, Schemas["JobResultResponse"]>();
    let pending = [...jobIds];
    while (pending.length > 0) {
      const stillPending: string[] = [];
      for (const jobId of pending) {
        const result = await this.jobs.getJobResult(jobId);
        if (TERMINAL_JOB_STATUSES.has(result.status)) {
          results.set(jobId, result);
        } else {
          stillPending.push(jobId);
        }
      }
      pending = stillPending;
      if (pending.length === 0) break;
      if (Date.now() >= deadline) {
        throw new JobTimeoutError(pending[0], timeoutMs);
      }
      await sleep(pollIntervalMs);
    }
    return results;
  }

  /**
   * Submit `body` to `POST /v1/documents/async` and poll until it finishes.
   *
   * Convenience composing `documents.processDocumentsBackground` and
   * `waitForJob`, mirroring xberg-sdks' `extract_and_wait`. For a batch small
   * enough to process inline, prefer `documents.processDocuments` instead —
   * this exists for batches you would otherwise have to submit-then-poll by
   * hand.
   */
  async extractAndWait(
    body: Schemas["ProcessDocumentsRequest"],
    options: WaitOptions = {},
  ): Promise<Schemas["JobResultResponse"]> {
    const job = await this.documents.processDocumentsBackground(body);
    return this.waitForJob(job.job_id, options);
  }
}
