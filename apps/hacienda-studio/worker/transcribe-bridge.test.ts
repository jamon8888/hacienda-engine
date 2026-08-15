import { describe, expect, it, vi } from "vitest";
import { TranscriptionRequestBridge } from "./transcribe-bridge";
import type { TranscriptionConfig, TranscriptionResult } from "../lib/transcription/types";

const CONFIG: TranscriptionConfig = { modelSize: "tiny.en", task: "transcribe" };

function result(overrides: Partial<TranscriptionResult> = {}): TranscriptionResult {
  return {
    text: "hello",
    segments: [],
    language: "en",
    duration: 0,
    metadata: { durationMs: 0, sampleRateHz: 16000, channels: 1, codec: "whisper-wasm" },
    ...overrides,
  };
}

describe("TranscriptionRequestBridge (worker-side request/response correlation map)", () => {
  it("posts a transcribe-request carrying the audio bytes as a transferable, and resolves the matching promise when resolve() is called", async () => {
    const post = vi.fn();
    const bridge = new TranscriptionRequestBridge(post);
    const audioBytes = new Uint8Array([1, 2, 3]);

    const promise = bridge.request("meeting.wav", audioBytes, "audio/wav", CONFIG);

    expect(post).toHaveBeenCalledTimes(1);
    const [message, transfer] = post.mock.calls[0];
    expect(message).toMatchObject({
      type: "transcribe-request",
      file: "meeting.wav",
      mimeType: "audio/wav",
      config: CONFIG,
    });
    expect(typeof message.requestId).toBe("string");
    expect(transfer).toEqual([audioBytes.buffer]);

    bridge.resolve(message.requestId, result({ text: "hi there" }));
    await expect(promise).resolves.toEqual(result({ text: "hi there" }));
  });

  it("rejects the matching promise when reject() is called", async () => {
    const post = vi.fn();
    const bridge = new TranscriptionRequestBridge(post);

    const promise = bridge.request("meeting.wav", new Uint8Array(), "audio/wav", CONFIG);
    const { requestId } = post.mock.calls[0][0];

    bridge.reject(requestId, "Whisper Web is not supported: `window` is not defined");
    await expect(promise).rejects.toThrow(/window` is not defined/);
  });

  it("assigns distinct requestIds to concurrent requests and resolves each independently, in either order", async () => {
    const post = vi.fn();
    const bridge = new TranscriptionRequestBridge(post);

    const p1 = bridge.request("a.wav", new Uint8Array(), "audio/wav", CONFIG);
    const p2 = bridge.request("b.wav", new Uint8Array(), "audio/wav", CONFIG);

    const id1 = post.mock.calls[0][0].requestId;
    const id2 = post.mock.calls[1][0].requestId;
    expect(id1).not.toBe(id2);

    // Resolve out of request order — the map must correlate by id, not by call order.
    bridge.resolve(id2, result({ text: "b" }));
    bridge.resolve(id1, result({ text: "a" }));

    await expect(p1).resolves.toEqual(result({ text: "a" }));
    await expect(p2).resolves.toEqual(result({ text: "b" }));
  });

  it("a transcription failure for one file does not affect another in-flight request", async () => {
    const post = vi.fn();
    const bridge = new TranscriptionRequestBridge(post);

    const ok = bridge.request("good.wav", new Uint8Array(), "audio/wav", CONFIG);
    const bad = bridge.request("bad.wav", new Uint8Array(), "audio/wav", CONFIG);
    const [okId, badId] = post.mock.calls.map((c) => c[0].requestId);

    bridge.reject(badId, "No content extracted");
    bridge.resolve(okId, result());

    await expect(ok).resolves.toEqual(result());
    await expect(bad).rejects.toThrow(/No content extracted/);
  });

  it("ignores resolve()/reject() for an unknown or already-settled requestId instead of throwing", async () => {
    const post = vi.fn();
    const bridge = new TranscriptionRequestBridge(post);

    expect(() => bridge.resolve("no-such-id", result())).not.toThrow();
    expect(() => bridge.reject("no-such-id", "boom")).not.toThrow();

    const promise = bridge.request("meeting.wav", new Uint8Array(), "audio/wav", CONFIG);
    const { requestId } = post.mock.calls[0][0];
    bridge.resolve(requestId, result());

    // A second, late reply for the same id (should never happen, but must not resurface
    // as a crash or silently re-resolve an already-settled promise) is a no-op.
    expect(() => bridge.resolve(requestId, result({ text: "late" }))).not.toThrow();
    await expect(promise).resolves.toEqual(result());
  });

  it("rejects with a timeout error if no response ever arrives, so one hung request cannot hang a sequential caller forever", async () => {
    vi.useFakeTimers();
    try {
      const post = vi.fn();
      const bridge = new TranscriptionRequestBridge(post, 1000);
      const promise = bridge.request("meeting.wav", new Uint8Array(), "audio/wav", CONFIG);

      const assertion = expect(promise).rejects.toThrow(/timed out after 1000ms/i);
      await vi.advanceTimersByTimeAsync(1000);
      await assertion;
    } finally {
      vi.useRealTimers();
    }
  });

  it("a late response after timeout is ignored rather than throwing an unhandled rejection", async () => {
    vi.useFakeTimers();
    try {
      const post = vi.fn();
      const bridge = new TranscriptionRequestBridge(post, 1000);
      const promise = bridge.request("meeting.wav", new Uint8Array(), "audio/wav", CONFIG);
      const { requestId } = post.mock.calls[0][0];

      const assertion = expect(promise).rejects.toThrow(/timed out/i);
      await vi.advanceTimersByTimeAsync(1000);
      await assertion;

      expect(() => bridge.resolve(requestId, result())).not.toThrow();
    } finally {
      vi.useRealTimers();
    }
  });
});
