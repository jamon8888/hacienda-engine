import type { TranscriptionConfig, TranscriptionResult } from "../lib/transcription/types";

/**
 * Worker-side half of the whisper transcription bridge.
 *
 * `WhisperBridge` (`lib/transcription/whisper-bridge.ts`) wraps `@remotion/whisper-web`,
 * whose own `canUseWhisperWeb()` requires `typeof window !== "undefined"` — a real browser
 * main thread, not a Web Worker (which has `self`, not `window`). Every `load()`/
 * `transcribeAudio()` call used to throw synchronously, unconditionally, in every
 * environment, because `pipeline.ts` (a Worker module) constructed and called
 * `WhisperBridge` directly (see `tests/e2e/audio.spec.ts`'s prior "known bug" tests, and the
 * comment this replaces above `processFiles`). The fix is architectural, not a patch:
 * `WhisperBridge` now lives and runs only on the main thread (`App.tsx`), and this worker
 * requests a transcription and awaits the answer over `postMessage` — the standard
 * request/response RPC pattern for worker↔main-thread work, correlated by `requestId`.
 *
 * `timeoutMs` exists for the same reason Real bug #2 (see `processFiles`'s comment on the
 * old `whisperBridge.load()` preload) mattered: `request()` is awaited inline inside
 * `processFile`, itself inside `processFiles`' *sequential* per-file loop — a `request()`
 * that never settles would hang not just its own file but every file queued after it. A
 * dropped or lost response (the main thread crashed, navigated away, or a bug in `App.tsx`'s
 * handler swallowed the reply) must still fail this one file, through the loop's existing
 * per-file try/catch, rather than stall the batch forever. The default is generous on
 * purpose: a cold "small" model (~769MB, see `@remotion/whisper-web`'s `SIZES`) over a slow
 * connection plus wasm/CPU transcription of a max-size (50MB — `asset-loader.ts`'s
 * `validateFile`) upload can legitimately take that long.
 */
const DEFAULT_TIMEOUT_MS = 15 * 60 * 1000;

interface PendingRequest {
  resolve: (result: TranscriptionResult) => void;
  reject: (error: Error) => void;
  timer: ReturnType<typeof setTimeout>;
}

export class TranscriptionRequestBridge {
  private readonly pending = new Map<string, PendingRequest>();
  private nextId = 0;

  /**
   * `post` is injected rather than calling `self.postMessage` directly so this class has no
   * dependency on a Worker global — it is a plain correlation map over an abstract
   * "send a message" function, which is what makes it unit-testable under Node/vitest
   * without the `self` polyfill `pipeline.test.ts` needs for the module that assigns
   * `self.onmessage`.
   */
  constructor(
    private readonly post: (message: unknown, transfer?: Transferable[]) => void,
    private readonly timeoutMs: number = DEFAULT_TIMEOUT_MS,
  ) {}

  /**
   * Sends a `transcribe-request` for `file` and returns a `Promise` that settles when the
   * matching `transcribe-response` arrives — via `resolve`/`reject` below, called from the
   * worker's `self.onmessage` — or when `timeoutMs` elapses first, whichever comes first.
   *
   * `audioBytes`' backing `ArrayBuffer` is transferred, not cloned: `input.bytes` (the
   * `FileInput` this view wraps) is not read again for this file once transcription has been
   * requested, so detaching it here costs nothing and avoids copying up to 50MB per file.
   */
  request(
    file: string,
    audioBytes: Uint8Array<ArrayBuffer>,
    mimeType: string,
    config: TranscriptionConfig,
  ): Promise<TranscriptionResult> {
    const requestId = `transcribe-${++this.nextId}`;
    return new Promise<TranscriptionResult>((resolve, reject) => {
      const timer = setTimeout(() => {
        this.pending.delete(requestId);
        reject(
          new Error(
            `Transcription request for "${file}" timed out after ${this.timeoutMs}ms waiting for a response from the main thread`,
          ),
        );
      }, this.timeoutMs);
      this.pending.set(requestId, { resolve, reject, timer });
      this.post(
        { type: "transcribe-request", requestId, file, audioBytes, mimeType, config },
        [audioBytes.buffer],
      );
    });
  }

  /**
   * Resolves the pending `request()` matching `requestId`. A response for an id that is no
   * longer pending — already timed out, or (should never happen, but must not crash the
   * worker if it does) a duplicate reply — is ignored rather than thrown: there is no
   * listener left to deliver it to.
   */
  resolve(requestId: string, result: TranscriptionResult): void {
    const entry = this.pending.get(requestId);
    if (!entry) return;
    this.pending.delete(requestId);
    clearTimeout(entry.timer);
    entry.resolve(result);
  }

  /** Same idempotency contract as `resolve`. */
  reject(requestId: string, message: string): void {
    const entry = this.pending.get(requestId);
    if (!entry) return;
    this.pending.delete(requestId);
    clearTimeout(entry.timer);
    entry.reject(new Error(message));
  }
}
