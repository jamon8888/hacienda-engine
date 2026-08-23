import { describe, expect, it, vi, beforeEach, afterEach } from "vitest";
import type { FileInput } from "./types";
import { DEFAULT_CONFIG } from "./types";
import { WorkerPool } from "./worker-pool";

/**
 * Stands in for the real `worker/pipeline.ts` Worker: each worker holds a
 * ~600MB GLiNER2 model in production, so the whole point of this test is to
 * prove `WorkerPool` only constructs as many of these as a batch actually
 * needs — never spawns them eagerly on `initialize()`, never spawns more than
 * `poolSize`.
 */
class MockWorker {
  static instances: MockWorker[] = [];
  // Regression fixture: makes a newly constructed worker never answer "init",
  // so `sendInit`'s 120s timeout is the only thing that ever settles it —
  // exercises the same timeout path a real slow/stuck wasm compile would hit,
  // without the test actually waiting 120 real seconds (see the fake-timer
  // test below).
  static hangOnInit = false;
  onmessage: ((event: MessageEvent) => void) | null = null;
  onerror: ((event: { message: string }) => void) | null = null;
  terminated = false;
  private listeners: Array<(event: MessageEvent) => void> = [];

  constructor(_url: URL, public options: { name?: string }) {
    MockWorker.instances.push(this);
  }

  // Real DOM Workers fire both the `.onmessage` property and any
  // `addEventListener("message", ...)` listeners independently for the same
  // message — `sendInit`'s "ready" handshake uses addEventListener while
  // `WorkerPool` itself sets `.onmessage` directly, so this mock has to
  // dispatch to both, not conflate them into one slot.
  addEventListener(_type: "message", handler: (event: MessageEvent) => void): void {
    this.listeners.push(handler);
  }

  removeEventListener(_type: "message", handler: (event: MessageEvent) => void): void {
    this.listeners = this.listeners.filter((l) => l !== handler);
  }

  private dispatch(data: unknown): void {
    const event = { data } as MessageEvent;
    this.onmessage?.(event);
    for (const listener of this.listeners) listener(event);
  }

  postMessage(data: any): void {
    if (data.type === "init") {
      if (MockWorker.hangOnInit) return;
      queueMicrotask(() => this.dispatch({ type: "ready" }));
      return;
    }
    if (data.type === "process") {
      queueMicrotask(() => {
        for (const file of data.files) {
          this.dispatch({
            type: "file-complete",
            name: file.name,
            entities: [],
            piiFindings: [],
            // `WorkerPool.processFiles` sorts results back into input order keyed on
            // `frontmatter.source` (the *input* file name — `.name` above is the
            // *output* document name), so the mock needs it too.
            frontmatter: { source: file.name },
          });
        }
        this.dispatch({ type: "batch-complete" });
      });
    }
  }

  terminate(): void {
    this.terminated = true;
  }
}

function fileInput(name: string): FileInput {
  return { name, bytes: new ArrayBuffer(0), type: "application/pdf" };
}

describe("WorkerPool", () => {
  beforeEach(() => {
    MockWorker.instances = [];
    MockWorker.hangOnInit = false;
    vi.stubGlobal("Worker", MockWorker);
  });

  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it("does not spawn any workers on initialize() alone", async () => {
    const pool = new WorkerPool({ poolSize: 3 });
    await pool.initialize();
    expect(MockWorker.instances).toHaveLength(0);
  });

  it("spawns exactly one worker for a single-file batch, not poolSize", async () => {
    const pool = new WorkerPool({ poolSize: 3 });
    await pool.initialize();

    const results = await pool.processFiles([fileInput("a.pdf")], DEFAULT_CONFIG);

    expect(MockWorker.instances).toHaveLength(1);
    expect(results.map((r) => r.name)).toEqual(["a.pdf"]);
  });

  it("grows to poolSize when a later batch needs more workers, reusing the first", async () => {
    const pool = new WorkerPool({ poolSize: 3 });
    await pool.initialize();

    await pool.processFiles([fileInput("a.pdf")], DEFAULT_CONFIG);
    expect(MockWorker.instances).toHaveLength(1);
    const firstWorker = MockWorker.instances[0];

    const results = await pool.processFiles(
      [fileInput("b.pdf"), fileInput("c.pdf"), fileInput("d.pdf"), fileInput("e.pdf")],
      DEFAULT_CONFIG,
    );

    // Capped at poolSize (3), never one worker per file (4).
    expect(MockWorker.instances).toHaveLength(3);
    // The worker from the first batch is reused, not discarded and rebuilt.
    expect(MockWorker.instances[0]).toBe(firstWorker);
    expect(results.map((r) => r.name).sort()).toEqual(["b.pdf", "c.pdf", "d.pdf", "e.pdf"]);
  });

  it("never exceeds poolSize even for a very large batch", async () => {
    const pool = new WorkerPool({ poolSize: 3 });
    await pool.initialize();

    const files = Array.from({ length: 10 }, (_, i) => fileInput(`file-${i}.pdf`));
    const results = await pool.processFiles(files, DEFAULT_CONFIG);

    expect(MockWorker.instances).toHaveLength(3);
    expect(results).toHaveLength(10);
  });

  /**
   * Regression: `growPool`'s `this.growing` was never reset after a failed
   * attempt — a `sendInit` timeout left it a permanently rejected promise, so
   * every later `processFiles()` call re-awaited that same stale rejection and
   * failed instantly, wedging the pool for the rest of the session with no
   * self-heal short of a page reload. The failed worker was also never
   * `.terminate()`'d, leaking it. Uses fake timers to exercise the real 120s
   * `sendInit` timeout without the test actually waiting 120 real seconds.
   */
  it("recovers after a worker init timeout instead of permanently wedging the pool", async () => {
    vi.useFakeTimers();
    try {
      const pool = new WorkerPool({ poolSize: 1 });
      await pool.initialize();

      MockWorker.hangOnInit = true;
      const failingBatch = pool.processFiles([fileInput("a.pdf")], DEFAULT_CONFIG);
      const assertion = expect(failingBatch).rejects.toThrow("Worker init timeout");
      await vi.advanceTimersByTimeAsync(120_000);
      await assertion;

      expect(MockWorker.instances).toHaveLength(1);
      expect(MockWorker.instances[0].terminated).toBe(true);

      // The real assertion: a later attempt must succeed rather than
      // immediately re-throwing the first attempt's stale rejection.
      MockWorker.hangOnInit = false;
      const results = await pool.processFiles([fileInput("b.pdf")], DEFAULT_CONFIG);
      expect(results.map((r) => r.name)).toEqual(["b.pdf"]);
      expect(MockWorker.instances).toHaveLength(2);
    } finally {
      vi.useRealTimers();
    }
  });
});
