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
  error?: { code?: string; message?: string };
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
    return data as T;
  }

  let code: string | undefined;
  let message = `request failed with status ${response.status}`;
  const body = result.error as ErrorEnvelope | undefined;
  if (body?.error) {
    code = body.error.code;
    message = body.error.message ?? message;
  }

  throw new HaciendaApiError(response.status, code, message);
}
