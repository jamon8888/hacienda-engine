/**
 * Starts a real `hacienda serve` instance for the test session. Per
 * `testing-anti-patterns` (never mock what you don't own): every test in
 * this suite exercises `HaciendaClient` against a live server built from the
 * same checkout, not a mocked fetch.
 */

import { spawn, type ChildProcess } from "node:child_process";
import { createServer } from "node:net";
import path from "node:path";
import { fileURLToPath } from "node:url";

const REPO_ROOT = path.resolve(
  path.dirname(fileURLToPath(import.meta.url)),
  "..",
  "..",
  "..",
);

async function freePort(): Promise<number> {
  return new Promise((resolve, reject) => {
    const server = createServer();
    server.listen(0, "127.0.0.1", () => {
      const address = server.address();
      if (address === null || typeof address === "string") {
        reject(new Error("could not determine a free port"));
        return;
      }
      const { port } = address;
      server.close(() => resolve(port));
    });
  });
}

async function waitForHealth(baseUrl: string): Promise<void> {
  for (let attempt = 0; attempt < 50; attempt += 1) {
    try {
      const response = await fetch(`${baseUrl}/health`);
      if (response.ok) return;
    } catch {
      // not up yet
    }
    await new Promise((resolve) => setTimeout(resolve, 200));
  }
  throw new Error("hacienda serve did not become ready in time");
}

export interface HaciendaServer {
  baseUrl: string;
  stop(): void;
}

/** Read the base URL of the server `tests/global-setup.ts` started for this run. */
export function getTestBaseUrl(): string {
  const baseUrl = process.env.HACIENDA_TEST_BASE_URL;
  if (!baseUrl) {
    throw new Error(
      "HACIENDA_TEST_BASE_URL is not set — tests must run through vitest's globalSetup (tests/global-setup.ts), not standalone",
    );
  }
  return baseUrl;
}

export async function startHacienda(): Promise<HaciendaServer> {
  await new Promise<void>((resolve, reject) => {
    const build = spawn("cargo", ["build", "-p", "hacienda-cli"], {
      cwd: REPO_ROOT,
      stdio: "inherit",
    });
    build.on("error", reject);
    build.on("exit", (code) =>
      code === 0 ? resolve() : reject(new Error(`cargo build exited ${code}`)),
    );
  });

  const port = await freePort();
  const baseUrl = `http://127.0.0.1:${port}`;
  const proc: ChildProcess = spawn(
    path.join(REPO_ROOT, "target", "debug", "hacienda"),
    ["serve", "--bind", `127.0.0.1:${port}`],
    { cwd: REPO_ROOT, stdio: "ignore" },
  );
  proc.on("error", (error) => {
    console.error("hacienda serve failed to start", error);
  });

  try {
    await waitForHealth(baseUrl);
  } catch (error) {
    proc.kill("SIGKILL");
    throw error;
  }

  return {
    baseUrl,
    stop() {
      proc.kill();
    },
  };
}
