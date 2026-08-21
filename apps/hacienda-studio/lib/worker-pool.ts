// Worker pool manager for cross-document parallelism (Tier 2.1)
// Distributes files across multiple workers to maximize CPU utilization
// Tier 2.2 (cold-start Wasm caching) is wired into `lib/pii-engine.ts`'s
// `initPiiEngine`, where the actual hacienda_wasm_bg.wasm fetch happens.

import type { FileInput, ProcessedFile, ProgressUpdate, AppConfig } from "./types";

export interface WorkerPoolConfig {
  /** Number of workers in the pool (default: conservative 3) */
  poolSize: number;
  /** Maximum files to queue per worker before backpressure */
  maxQueueDepth: number;
}

export interface WorkerTask {
  id: string;
  files: FileInput[];
  config: AppConfig;
  results: ProcessedFile[];
  resolve: (results: ProcessedFile[]) => void;
  reject: (error: Error) => void;
}

interface WorkerInstance {
  worker: Worker;
  busy: boolean;
  currentTask: WorkerTask | null;
  filesProcessed: number;
  pendingFiles: number;
}

const DEFAULT_CONFIG: WorkerPoolConfig = {
  // Safe-default floor for a caller that omits `poolSize` entirely. `App.tsx` always
  // passes an explicit value computed from `lib/device-tier.ts`'s `poolSizeForTier` —
  // this fallback shouldn't assume anything about the caller's RAM (each worker holds
  // a ~600MB model), so it stays at the conservative floor rather than the old flat 3.
  poolSize: 1,
  maxQueueDepth: 10,
};

/**
 * Worker pool for parallel document processing.
 * 
 * Each worker holds a full GLiNER2 model instance (~600MB), so pool size
 * is limited by available RAM rather than CPU cores.
 * 
 * Features:
 * - Tier 2.1: Cross-document parallelism via worker pool
 * - Tier 2.2: Cold-start caching of Wasm modules
 * 
 * Usage:
 *   const pool = new WorkerPool({ poolSize: 3 });
 *   await pool.initialize(transcribeHandler);
 *   const results = await pool.processFiles(files, config);
 *   await pool.terminate();
 */
export class WorkerPool {
  private workers: WorkerInstance[] = [];
  private config: WorkerPoolConfig;
  private taskQueue: WorkerTask[] = [];
  private initializing: Promise<void> | null = null;
  private growing: Promise<void> | null = null;

  // Transcription handler (main thread only - for whisper). `respond` must be called
  // exactly once with the result so the reply reaches the *originating* pool worker —
  // there is no single "the worker" anymore now that there are `poolSize` of them.
  private transcribeHandler?: (
    data: {
      requestId: string;
      file: string;
      audioBytes: Uint8Array<ArrayBuffer>;
      mimeType: string;
      config: any;
    },
    respond: (response: { result?: unknown; error?: string }) => void,
  ) => Promise<void>;

  // Progress tracking
  private onProgress?: (update: ProgressUpdate) => void;
  private onFileComplete?: (file: ProcessedFile) => void;
  private onError?: (file: string, error: string) => void;
  private onWarning?: (message: string) => void;
  private onBatchComplete?: () => void;

  constructor(config: Partial<WorkerPoolConfig> = {}) {
    this.config = { ...DEFAULT_CONFIG, ...config };
  }

  /**
   * Register the transcription handler. Does not spawn any workers — each one holds a
   * full GLiNER2 model (~600MB), so spawning `poolSize` of them up front (e.g. on app
   * mount, before any file is even chosen) reserved ~1.8GB of baseline memory
   * regardless of how many files the user actually uploads. Workers are now spawned
   * lazily by `growPool()`, sized to what each batch actually needs.
   */
  async initialize(
    transcribeHandler?: (
      data: {
        requestId: string;
        file: string;
        audioBytes: Uint8Array<ArrayBuffer>;
        mimeType: string;
        config: any;
      },
      respond: (response: { result?: unknown; error?: string }) => void,
    ) => Promise<void>,
  ): Promise<void> {
    if (this.initializing) return this.initializing;
    this.transcribeHandler = transcribeHandler;
    this.initializing = Promise.resolve();
    return this.initializing;
  }

  /**
   * Spawns workers until the pool has at least `target` of them (capped at
   * `poolSize`), reusing any already running. Called from `processFiles()` sized to
   * that batch's file count — a single-file upload only ever pays for one ~600MB
   * model, not `poolSize` of them.
   */
  private async growPool(target: number): Promise<void> {
    if (this.growing) await this.growing;

    const capped = Math.min(target, this.config.poolSize);
    if (this.workers.length >= capped) return;

    this.growing = (async () => {
      const start = this.workers.length;
      const toSpawn = capped - start;
      console.log(`[WorkerPool] Growing pool from ${start} to ${capped} workers...`);

      const initPromises = Array.from({ length: toSpawn }, async (_, offset) => {
        const i = start + offset;
        // Tier 2.2's cold-start caching applies to the compiled hacienda_wasm_bg.wasm
        // binary that each worker fetches internally during its own `init()` handshake
        // (see `lib/pii-engine.ts`'s `initPiiEngine`), not to this `worker/pipeline.ts`
        // module script — a Worker must always be constructed from its real module URL.
        const worker = new Worker(new URL("../worker/pipeline.ts", import.meta.url), {
          type: "module",
          name: `hacienda-worker-${i}`
        });

        const instance: WorkerInstance = {
          worker,
          busy: false,
          currentTask: null,
          filesProcessed: 0,
          pendingFiles: 0,
        };

        // Set up message handlers
        worker.onmessage = (event: MessageEvent) => {
          this.handleWorkerMessage(instance, event);
        };

        worker.onerror = (error) => {
          console.error(`[WorkerPool] Worker ${i} error:`, error);
          this.handleWorkerError(instance, error.message);
        };

        // Initialize the worker
        await this.sendInit(worker);

        this.workers.push(instance);
        console.log(`[WorkerPool] Worker ${i} ready`);
      });

      await Promise.all(initPromises);
      console.log(`[WorkerPool] Pool now has ${this.workers.length} workers`);
    })();

    return this.growing;
  }

  /**
   * Send init message to worker and wait for ready
   */
  private sendInit(worker: Worker): Promise<void> {
    return new Promise((resolve, reject) => {
      const timeout = setTimeout(() => reject(new Error("Worker init timeout")), 120000);
      
      const handler = (event: MessageEvent) => {
        if (event.data.type === "ready") {
          clearTimeout(timeout);
          worker.removeEventListener("message", handler);
          resolve();
        }
      };
      
      worker.addEventListener("message", handler);
      worker.postMessage({ type: "init" });
    });
  }

  /**
   * Process multiple files in parallel across the worker pool.
   * Distributes files round-robin across available workers.
   */
  async processFiles(files: FileInput[], config: AppConfig): Promise<ProcessedFile[]> {
    await this.initialize();

    if (files.length === 0) return [];

    // Only pay for as many workers (and their ~600MB models) as this batch can
    // actually use — one file never needs three workers.
    await this.growPool(files.length);

    console.log(`[WorkerPool] Processing ${files.length} files across ${this.workers.length} workers`);

    // Distribute files round-robin for better load balancing
    const workerFiles: FileInput[][] = Array.from(
      { length: this.workers.length },
      () => []
    );
    
    files.forEach((file, i) => {
      workerFiles[i % this.workers.length].push(file);
    });

    // Filter out empty assignments
    const assignments = workerFiles
      .map((files, i) => ({ workerIndex: i, files }))
      .filter(({ files }) => files.length > 0);

    // Create tasks with result arrays
    const tasks = assignments.map(({ workerIndex, files: assignedFiles }) => ({
      workerIndex,
      task: {
        id: `${Date.now()}-${Math.random()}`,
        files: assignedFiles,
        config,
        results: [] as ProcessedFile[],
        resolve: (_results: ProcessedFile[]) => {}, // Will be set below
        reject: (_error: Error) => {}, // Will be set below
      },
    }));

    // Set up resolve/reject for each task
    const promises = tasks.map(({ task }) => 
      new Promise<ProcessedFile[]>((resolve, reject) => {
        task.resolve = resolve;
        task.reject = reject;
      })
    );

    // Start processing on each worker
    tasks.forEach(({ workerIndex, task }) => {
      const instance = this.workers[workerIndex];
      instance.busy = true;
      instance.pendingFiles = task.files.length;
      instance.currentTask = task;

      // Send process message
      instance.worker.postMessage({
        type: "process",
        files: task.files,
        config,
      });
    });

    // Wait for all tasks to complete
    const allResults = await Promise.all(promises);
    
    // Flatten results preserving order (by file index)
    const fileResults = allResults.flat();
    
    // Sort by original file order
    const fileOrder = new Map(files.map((f, i) => [f.name, i]));
    fileResults.sort((a, b) => 
      (fileOrder.get(a.name) ?? 0) - (fileOrder.get(b.name) ?? 0)
    );

    return fileResults;
  }

  /**
   * Handle messages from workers
   */
  private handleWorkerMessage(instance: WorkerInstance, event: MessageEvent): void {
    const { type, ...data } = event.data;

    switch (type) {
      case "progress":
        this.onProgress?.(data as ProgressUpdate);
        break;
        
      case "transcribe-request":
        // Forward transcription request to main thread handler
        if (this.transcribeHandler) {
          void this.transcribeRequest(instance, data);
        }
        break;
        
      case "file-complete":
        instance.filesProcessed++;
        instance.pendingFiles--;
        const result = data as ProcessedFile;
        
        // Add to current task's results
        if (instance.currentTask) {
          instance.currentTask.results.push(result);
        }
        
        this.onFileComplete?.(result);
        break;
        
      case "warning":
        this.onWarning?.(data.message);
        break;
        
      case "error":
        this.onError?.(data.file, data.message);
        break;
        
      case "batch-complete":
        // Worker finished its assigned batch - resolve the task
        if (instance.currentTask) {
          instance.currentTask.resolve(instance.currentTask.results);
          instance.currentTask = null;
        }
        instance.busy = false;
        instance.pendingFiles = 0;
        break;
        
      case "zip-ready":
        // Not used in pool mode
        break;
    }
  }

  /**
   * Handle transcription request from worker
   */
  private async transcribeRequest(instance: WorkerInstance, data: any): Promise<void> {
    const respond = (response: { result?: unknown; error?: string }) => {
      instance.worker.postMessage({
        type: "transcribe-response",
        requestId: data.requestId,
        ...response,
      });
    };
    try {
      await this.transcribeHandler?.(data, respond);
    } catch (error) {
      console.error("[WorkerPool] Transcription handler error:", error);
      respond({ error: error instanceof Error ? error.message : String(error) });
    }
  }

  private handleWorkerError(instance: WorkerInstance, error: string): void {
    if (instance.currentTask) {
      instance.currentTask.reject(new Error(`Worker error: ${error}`));
      instance.currentTask = null;
    }
    // A wasm trap (e.g. "RuntimeError: unreachable executed") leaves that worker's
    // wasm linear memory permanently corrupted — every later call into it just
    // traps again, which is why one bad file can otherwise produce a burst of
    // identical errors as other in-flight work in the same worker unwinds. Kill
    // the worker outright instead of marking it merely idle, so the pool's next
    // `growPool()` spawns a clean replacement rather than routing more files at
    // an instance that can never recover.
    instance.worker.terminate();
    instance.busy = false;
    this.workers = this.workers.filter((w) => w !== instance);
  }

  /**
   * Set progress callback
   */
  setProgressHandler(handler: (update: ProgressUpdate) => void): void {
    this.onProgress = handler;
  }

  /**
   * Set file complete callback
   */
  setFileCompleteHandler(handler: (file: ProcessedFile) => void): void {
    this.onFileComplete = handler;
  }

  /**
   * Set error callback
   */
  setErrorHandler(handler: (file: string, error: string) => void): void {
    this.onError = handler;
  }

  /**
   * Set warning callback
   */
  setWarningHandler(handler: (message: string) => void): void {
    this.onWarning = handler;
  }

  /**
   * Set batch complete callback
   */
  setBatchCompleteHandler(handler: () => void): void {
    this.onBatchComplete = handler;
  }

  /**
   * Get pool statistics
   */
  getStats(): { totalWorkers: number; busyWorkers: number; totalFilesProcessed: number } {
    return {
      totalWorkers: this.workers.length,
      busyWorkers: this.workers.filter(w => w.busy).length,
      totalFilesProcessed: this.workers.reduce((sum, w) => sum + w.filesProcessed, 0),
    };
  }

  /**
   * Terminate all workers and clean up
   */
  async terminate(): Promise<void> {
    console.log("[WorkerPool] Terminating all workers...");
    
    await Promise.all(
      this.workers.map(async (instance) => {
        instance.worker.terminate();
      })
    );
    
    this.workers = [];
    this.initializing = null;
    this.growing = null;
    console.log("[WorkerPool] All workers terminated");
  }
}

/**
 * Factory function to create and initialize a worker pool
 */
export async function createWorkerPool(
  config?: Partial<WorkerPoolConfig>,
  transcribeHandler?: (
    data: {
      requestId: string;
      file: string;
      audioBytes: Uint8Array<ArrayBuffer>;
      mimeType: string;
      config: any;
    },
    respond: (response: { result?: unknown; error?: string }) => void,
  ) => Promise<void>,
): Promise<WorkerPool> {
  const pool = new WorkerPool(config);
  await pool.initialize(transcribeHandler);
  return pool;
}
