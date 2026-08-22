/**
 * Minimal DeepSeek Harness RPC client.
 *
 * The DeepSeek Harness exposes its whole agent backend as an HTTP RPC bus at
 * `/api/<method>`: you POST a `client-request` frame and receive a
 * `server-response`. Unary method calls go to `/api/<method>`; assistant
 * output then appears in the session's event log, which we read by polling
 * `/api/session.history` and re-publishing `assistant/chunk` text deltas as a
 * Vercel-AI-SDK stream (see `chat.ts`).
 *
 * Wire format (verified against the running harness at 127.0.0.1:3081):
 *   POST /api/session.create { type, rpcId, method, payload:{cwd} }          -> {sessionId}
 *   POST /api/session.prompt  { ..., payload:{sessionId, mode:'queue', content:[{type:'text',text}]} } -> {accepted:true}
 *   POST /api/session.history { ..., payload:{sessionId, maxMessages} }       -> {events:[{event:{type,data}}]}
 *   POST /api/session.cancel  { ..., payload:{sessionId} }                    -> {accepted:true}
 */
import type { Logger } from "./logger.js";

let rpcCounter = 0;

export interface HarnessOptions {
  /** Base URL of the DeepSeek Harness web server, e.g. http://127.0.0.1:3081 */
  baseUrl: string;
  /** Working directory passed to session.create. */
  cwd: string;
  logger?: Logger;
}

export class HarnessError extends Error {
  constructor(
    message: string,
    public readonly code: string,
    public readonly details: unknown,
  ) {
    super(message);
    this.name = "HarnessError";
  }
}

interface RpcResult {
  ok: boolean;
  value?: unknown;
  error?: { code: string; message: string; details?: unknown };
}

/**
 * Perform one unary RPC call: POST /api/<method> with a client-request frame.
 * Throws HarnessError on HTTP or `result.ok === false`.
 */
export async function callMethod<T>(opts: HarnessOptions, method: string, payload: unknown): Promise<T> {
  const rpcId = `adapter-${process.pid}-${++rpcCounter}`;
  let response: Response;
  try {
    response = await fetch(`${opts.baseUrl}/api/${method}`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ type: "client-request", rpcId, method, payload }),
    });
  } catch (err) {
    throw new HarnessError(`Cannot reach harness at ${opts.baseUrl}: ${(err as Error).message}`, "harness-unreachable", err);
  }
  if (!response.ok) {
    throw new HarnessError(
      `Harness returned HTTP ${response.status} for ${method}`,
      "harness-http",
      await response.text().catch(() => ""),
    );
  }
  const body = (await response.json()) as { type: string; rpcId: string; result: RpcResult };
  if (body.result?.ok !== true) {
    const e = body.result?.error ?? { code: "internal", message: "unknown error" };
    throw new HarnessError(`Harness RPC ${method} failed: ${e.message}`, e.code, e.details);
  }
  return body.result.value as T;
}

export interface SessionSummary {
  sessionId: string;
}

export async function createSession(opts: HarnessOptions): Promise<string> {
  const value = await callMethod<SessionSummary>(opts, "session.create", { cwd: opts.cwd });
  return value.sessionId;
}

export async function promptSession(opts: HarnessOptions, sessionId: string, text: string): Promise<void> {
  await callMethod<{ accepted: true }>(opts, "session.prompt", {
    sessionId,
    mode: "queue",
    content: [{ type: "text", text }],
  });
}

export interface HistoryEntry {
  event: {
    type: string;
    seq: number;
    time: number;
    data: Record<string, unknown>;
  };
}

export interface HistoryResult {
  events: HistoryEntry[];
  hasMore: boolean;
}

export async function sessionHistory(opts: HarnessOptions, sessionId: string, maxMessages = 200): Promise<HistoryResult> {
  return callMethod<HistoryResult>(opts, "session.history", { sessionId, maxMessages });
}

export async function cancelSession(opts: HarnessOptions, sessionId: string): Promise<void> {
  await callMethod<{ accepted: true }>(opts, "session.cancel", { sessionId }).catch(() => {});
}
