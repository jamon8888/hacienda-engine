/**
 * /api/chat handler: Vercel AI SDK "UI Message Stream" v1 SSE endpoint backed
 * by the DeepSeek Harness.
 *
 * useChat (@ai-sdk/react v4 / ai@7) consumes this exact wire format (verified
 * against the AI SDK source: readUIMessageStream + processUIMessageStream):
 *   Content-Type: text/event-stream
 *   x-vercel-ai-ui-message-stream: v1
 *
 *   data: {"type":"text-start","id":"<partId>"}
 *   data: {"type":"text-delta","id":"<partId>","delta":"<text>"}
 *   data: {"type":"text-end","id":"<partId>"}
 *   data: {"type":"finish","finishReason":"stop","messageMetadata":{...}}
 *   data: [DONE]
 *
 * A text-start MUST precede any text-delta for that id; the AI SDK throws if
 * a delta arrives for a missing part. On each request we create a fresh
 * harness session, submit the last user message via session.prompt, then poll
 * session.history and re-publish `assistant/chunk` text-delta events.
 */
import type { Request, Response } from "express";
import { createSession, promptSession, sessionHistory, cancelSession, type HarnessOptions } from "./harness.js";

export interface ChatRequest {
  messages?: { role: string; content: string | unknown }[];
}

const POLL_INTERVAL_MS = 250;
const POLL_TIMEOUT_MS = 180_000; // 3 minutes hard cap for one turn

function lastUserText(body: ChatRequest): string | null {
  const messages = body.messages ?? [];
  for (let i = messages.length - 1; i >= 0; i--) {
    const m = messages[i];
    if (m.role !== "user") continue;
    // content may be a plain string, an array of {type,text} parts, or the
    // message may carry a `parts` array — handle all three.
    const candidates: unknown[] = [];
    if (typeof m.content === "string") candidates.push({ text: m.content });
    else if (Array.isArray(m.content)) candidates.push(...(m.content as unknown[]));
    if (Array.isArray((m as { parts?: unknown }).parts)) {
      const possibleParts = (m as unknown as { parts: unknown[] }).parts as unknown[];
      candidates.push(...possibleParts);
    }
    const text = candidates
      .map((p) => {
        if (p && typeof p === "object" && "text" in (p as object)) {
          return (p as { text?: unknown }).text;
        }
        return undefined;
      })
      .filter((t): t is string => typeof t === "string" && t.length > 0)
      .join("\n");
    if (text.trim().length > 0) return text;
  }
  return null;
}

function sseJson(res: Response, value: unknown): void {
  res.write(`data: ${JSON.stringify(value)}\n\n`);
}

function sseDone(res: Response): void {
  res.write("data: [DONE]\n\n");
}

export function chatHandler(opts: HarnessOptions) {
  return async (req: Request, res: Response) => {
    const body = (req.body ?? {}) as ChatRequest;
    const userText = lastUserText(body);
    const partId = "text-" + Date.now().toString(36);

    res.status(200);
    res.setHeader("Content-Type", "text/event-stream; charset=utf-8");
    res.setHeader("Cache-Control", "no-cache, no-transform");
    res.setHeader("Connection", "keep-alive");
    res.setHeader("X-Accel-Buffering", "no");
    res.setHeader("x-vercel-ai-ui-message-stream", "v1");
    res.flushHeaders?.();

    if (!userText) {
      sseJson(res, { type: "error", errorText: "No user message provided" });
      sseDone(res);
      res.end();
      return;
    }

    let sessionId: string | null = null;
    let sentDeltas = 0;
    const startedAt = Date.now();
    const pollDeadline = startedAt + POLL_TIMEOUT_MS;

    try {
      sessionId = await createSession(opts);
      await promptSession(opts, sessionId, userText);

      // Begin the text part the moment the prompt is accepted so empty-spinner
      // states are avoided, then stream deltas as they appear.
      sseJson(res, { type: "text-start", id: partId });

      let lastSeq = -1;
      let finished = false;
      let sawFinishEvent = false;

      while (Date.now() < pollDeadline) {
        const history = await sessionHistory(opts, sessionId, 500);
        const events = history.events;

        let emittedThisRound = 0;
        for (const entry of events) {
          const event = entry.event;
          if (event.seq <= lastSeq) continue;
          const data = (event.data ?? {}) as Record<string, unknown>;
          const chunk = (data.chunk ?? data) as { type?: string; text?: string };

          if (event.type === "assistant/chunk") {
            if (chunk.type === "text-delta" && typeof chunk.text === "string" && chunk.text.length > 0) {
              sseJson(res, { type: "text-delta", id: partId, delta: chunk.text });
              sentDeltas++;
              emittedThisRound++;
            }
            if (chunk.type === "finish") {
              sawFinishEvent = true;
            }
          }
          if (event.type === "turn/end") {
            finished = true;
          }
          lastSeq = Math.max(lastSeq, event.seq);
        }

        if (emittedThisRound > 0) {
          opts.logger?.log(`[chat] streamed ${emittedThisRound} delta(s), total=${sentDeltas}`);
        }

        if (finished || sawFinishEvent) break;
        await sleep(POLL_INTERVAL_MS);
      }

      sseJson(res, { type: "text-end", id: partId });
      sseJson(res, {
        type: "finish",
        finishReason: finished || sawFinishEvent ? "stop" : "error",
        messageMetadata: { id: req.headers["x-request-id"] ?? undefined },
      });
      sseDone(res);
      res.end();
    } catch (err) {
      opts.logger?.error(`[chat] failed: ${(err as Error).stack ?? err}`);
      sseJson(res, { type: "error", errorText: (err as Error).message });
      sseDone(res);
      if (!res.writableEnded) res.end();
    } finally {
      if (sessionId) await cancelSession(opts, sessionId);
    }
  };
}

function sleep(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms));
}
