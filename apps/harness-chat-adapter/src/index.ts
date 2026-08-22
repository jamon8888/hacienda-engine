/**
 * harness-chat-adapter — standalone server exposing a Vercel AI SDK /api/chat
 * endpoint backed by the DeepSeek Harness RPC.
 *
 * Env:
 *   HARNESS_BASE_URL  default http://127.0.0.1:3081
 *   HARNESS_CWD       default process.cwd()
 *   ADAPTER_PORT      default 4001
 *   ADAPTER_HOST      default 127.0.0.1
 *
 * Run: npm run dev
 */
import express from "express";
import cors from "cors";
import { chatHandler } from "./chat.js";
import { HarnessError, callMethod } from "./harness.js";
import { consoleLogger, type Logger } from "./logger.js";

const baseUrl = process.env.HARNESS_BASE_URL ?? "http://127.0.0.1:3081";
const cwd = process.env.HARNESS_CWD ?? process.cwd();
const port = Number(process.env.ADAPTER_PORT ?? 4001);
const host = process.env.ADAPTER_HOST ?? "127.0.0.1";

const opts = { baseUrl, cwd, logger: consoleLogger as Logger };

const app = express();
app.use(cors());
app.use(express.json({ limit: "2mb" }));

app.get("/api/health", async (_req, res) => {
  try {
    const hostInfo = await callMethod<{ version?: string; provider?: string; model?: string }>(
      opts,
      "host.describe",
      {},
    );
    res.json({ ok: true, harness: hostInfo });
  } catch (err) {
    res.status(503).json({
      ok: false,
      error: err instanceof HarnessError ? { code: err.code, message: err.message } : String(err),
    });
  }
});

app.post("/api/chat", chatHandler(opts));

app.listen(port, host, () => {
  consoleLogger.log(`[adapter] listening on http://${host}:${port}`);
  consoleLogger.log(`[adapter] harness base URL: ${baseUrl} (cwd=${cwd})`);
  consoleLogger.log(`[adapter] POST /api/chat  → Vercel AI SDK stream backed by harness RPC`);
  consoleLogger.log(`[adapter] GET  /api/health`);
});
