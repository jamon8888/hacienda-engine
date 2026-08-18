/**
 * Raised for any non-2xx response from the hacienda API.
 *
 * Mirrors the wire error envelope hacienda-api sends on every error response
 * (`{"error": {"code": "...", "message": "..."}}`, see hacienda-api's
 * `error.rs`): `code` is the machine-readable snake_case string
 * (`"invalid_request"`, `"forbidden"`, ...), `message` is the client-safe
 * sentence. Both fall back to `undefined`/a generic message if the body could
 * not be parsed as that envelope.
 */
export class HaciendaApiError extends Error {
  readonly statusCode: number;
  readonly code: string | undefined;

  constructor(statusCode: number, code: string | undefined, message: string) {
    super(`HTTP ${statusCode} (${code ?? "unknown"}): ${message}`);
    this.name = "HaciendaApiError";
    this.statusCode = statusCode;
    this.code = code;
  }
}

interface ErrorEnvelope {
  error?: { code?: unknown; message?: unknown };
}

function isErrorEnvelope(value: unknown): value is ErrorEnvelope {
  return typeof value === "object" && value !== null;
}

function stringOrUndefined(value: unknown): string | undefined {
  return typeof value === "string" ? value : undefined;
}

/**
 * Return `data` on a 2xx response, throw {@link HaciendaApiError} otherwise.
 *
 * openapi-fetch never throws on a non-2xx status — it resolves `{ data,
 * error, response }` with `data` undefined and `error` set to the parsed (or
 * raw) body. Every wrapper method in `client.ts` routes its result through
 * this function specifically so a caller gets a real exception instead of
 * having to check `error` after every call.
 */
export async function unwrap<T>(result: {
  data?: T;
  error?: unknown;
  response: Response;
}): Promise<T> {
  const { data, response } = result;
  if (response.ok) {
    // `data` is legitimately undefined for 204 No Content responses
    // (revokeKey, deleteCollection, deletePreset — all `Promise<void>`), so
    // this stays permissive rather than throwing on an empty body.
    return data as T;
  }

  let code: string | undefined;
  let message = `request failed with status ${response.status}`;
  const body = result.error;
  if (
    isErrorEnvelope(body) &&
    typeof body.error === "object" &&
    body.error !== null
  ) {
    code = stringOrUndefined(body.error.code);
    message = stringOrUndefined(body.error.message) ?? message;
  }

  throw new HaciendaApiError(response.status, code, message);
}

/**
 * Thrown by `waitForJob`/`waitForJobs`/`extractAndWait` when a job has not
 * reached a terminal state (`succeeded` or `failed`) within the given
 * `timeoutMs`. The job itself is still running server-side; this only means
 * the client stopped waiting.
 */
export class JobTimeoutError extends Error {
  readonly jobId: string;
  readonly timeoutMs: number;

  constructor(jobId: string, timeoutMs: number) {
    super(`job ${jobId} did not reach a terminal state within ${timeoutMs}ms`);
    this.name = "JobTimeoutError";
    this.jobId = jobId;
    this.timeoutMs = timeoutMs;
  }
}
