// Worker pool manager for cross-document parallelism (Tier 2.1)
// Distributes files across multiple workers to maximize CPU utilization

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
  poolSize: 3, // Conservative: limited by RAM per worker (model ~600MB each)
  maxQueueDepth: 10,
};

/**
 * Worker pool for parallel document processing.
 * 
 * Each worker holds a full GLiNER2 model instance (~600MB), so pool size
 * is limited by available RAM rather than CPU cores.
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
  
  // Transcription handler (main thread only - for whisper)
  private transcribeHandler?: (data: {
    requestId: string;
    file: string;
    audioBytes: Uint8Array<ArrayBuffer>;
    mimeType: string;
    config: any;
  }) => Promise<void>;

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
   * Initialize the worker pool - creates and initializes all workers.
   * Must be called before processFiles().
   */
  async initialize(transcribeHandler?: (data: {
    requestId: string;
    file: string;
    audioBytes: Uint8Array<ArrayBuffer>;
    mimeType: string;
    config: any;
  }) => Promise<void>): Promise<void> {
    if (this.initializing) return this.initializing;

    this.transcribeHandler = transcribeHandler;

    this.initializing = (async () => {
      console.log(`[WorkerPool] Initializing ${this.config.poolSize} workers...`);
      
      const initPromises = Array.from({ length: this.config.poolSize }, async (_, i) => {
        const worker = new Worker(
          new URL("./worker/pipeline.ts", import.meta.url),
          { type: "module", name: `hacienda-worker-${i}` }
        );

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
      console.log(`[WorkerPool] All ${this.workers.length} workers initialized`);
    })();

    return this.initializing;
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
        resolve: () => {}, // Will be set below
        reject: () => {}, // Will be set below
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
        
        // Process next queued task
        // (Queue not implemented yet - could be added for future work)
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
    try {
      await this.transcribeHandler?.(data);
    } catch (error) {
      console.error("[WorkerPool] Transcription handler error:", error);
      // Send error response back to worker
      instance.worker.postMessage({
        type: "transcribe-response",
        requestId: data.requestId,
        error: error instanceof Error ? error.message : String(error),
      });
    }
  }

  private handleWorkerError(instance: WorkerInstance, error: string): void {
    if (instance.currentTask) {
      instance.currentTask.reject(new Error(`Worker error: ${error}`));
      instance.currentTask = null;
    }
    instance.busy = false;
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
    console.log("[WorkerPool] All workers terminated");
  }
}

/**
 * Factory function to create and initialize a worker pool
 */
export async function createWorkerPool(
  config?: Partial<WorkerPoolConfig>,
  transcribeHandler?: (data: {
    requestId: string;
    file: string;
    audioBytes: Uint8Array<ArrayBuffer>;
    mimeType: string;
    config: any;
  }) => Promise<void>
): Promise<WorkerPool> {
  const pool = new WorkerPool(config);
  await pool.initialize(transcribeHandler);
  return pool;
}
