import {
  transcribe,
  canUseWhisperWeb,
  resampleTo16Khz,
  downloadWhisperModel,
} from "@remotion/whisper-web";
import type {
  TranscriptionConfig,
  TranscriptionResult,
  TranscriptionSegment,
} from "./types";

/**
 * Must be constructed and called on the main thread only. `@remotion/whisper-web`'s
 * `canUseWhisperWeb()` gates on `typeof window !== "undefined"`, and `downloadWhisperModel`'s
 * IndexedDB cache plus `resampleTo16Khz`'s `AudioContext`-based decode are both main-thread
 * browser APIs — none of that exists inside a Web Worker (`self`, not `window`). Studio's
 * pipeline runs in a Worker (`worker/pipeline.ts`), so that worker never constructs this
 * class directly; it requests a transcription over `postMessage` and `App.tsx` is the one
 * `WhisperBridge` instance actually performing it — see `worker/transcribe-bridge.ts` for the
 * request/response bridge that connects the two.
 *
 * One instance should live for the life of the tab (a ref in `App.tsx`, not a per-request
 * throwaway): `load()` is idempotent per `modelSize` (see below), so reusing the instance
 * across every transcription in a batch — and across batches — is what avoids re-downloading
 * or re-initializing the model per file.
 */
export class WhisperBridge {
  private loaded = false;
  private modelSize: string = "tiny.en";

  async load(
    modelSize:
      | "tiny.en"
      | "tiny"
      | "base.en"
      | "base"
      | "small.en"
      | "small" = "tiny.en",
  ) {
    if (this.loaded && this.modelSize === modelSize) return;

    const { supported, detailedReason } = await canUseWhisperWeb(modelSize);
    if (!supported) {
      throw new Error(`Whisper Web is not supported: ${detailedReason}`);
    }

    console.log(`[WhisperBridge] Downloading model: ${modelSize}`);
    await downloadWhisperModel({
      model: modelSize,
      onProgress: ({ progress }) => {
        console.log(
          `[WhisperBridge] Downloading model (${Math.round(progress * 100)}%)...`,
        );
      },
    });

    this.loaded = true;
    this.modelSize = modelSize;
    console.log(`[WhisperBridge] Model ready: ${modelSize}`);
  }

  async transcribeAudio(
    // Not a bare Uint8Array: that widens to Uint8Array<ArrayBufferLike>, which
    // may be backed by a SharedArrayBuffer and is not a valid BlobPart.
    audioBytes: Uint8Array<ArrayBuffer>,
    mimeType: string,
    config: TranscriptionConfig,
    /**
     * Fires with a 0..1 fraction for each of the two phases below. Optional: the caller
     * (`App.tsx`, now that this class only ever runs on the main thread) decides whether to
     * surface it, e.g. into the existing per-file progress UI — this class itself has no
     * opinion on where progress is displayed.
     */
    onProgress?: (phase: "resample" | "transcribe", fraction: number) => void,
  ): Promise<TranscriptionResult> {
    if (!this.loaded) {
      await this.load(config.modelSize || "tiny.en");
    }

    console.log("[WhisperBridge] Resampling audio...");
    const file = new File([audioBytes], "audio", { type: mimeType });
    const channelWaveform = await resampleTo16Khz({
      file,
      onProgress: (p) => {
        console.log(
          `[WhisperBridge] Resampling audio (${Math.round(p * 100)}%)...`,
        );
        onProgress?.("resample", p);
      },
    });

    console.log("[WhisperBridge] Transcribing...");
    const { transcription } = await transcribe({
      channelWaveform,
      model: this.modelSize as any,
      onProgress: (p) => {
        console.log(
          `[WhisperBridge] Transcribing (${Math.round(p * 100)}%)...`,
        );
        onProgress?.("transcribe", p);
      },
    });

    const segments: TranscriptionSegment[] = transcription.map((t: any) => ({
      start: t.start ?? 0,
      end: t.end ?? 0,
      text: t.text ?? "",
      confidence: t.noSpeechProb ? 1 - t.noSpeechProb : undefined,
    }));

    const fullText = segments.map((s) => s.text).join(" ");

    return {
      text: fullText,
      segments,
      language: config.language || "en",
      duration: segments.length > 0 ? segments[segments.length - 1].end : 0,
      metadata: {
        durationMs: segments.length > 0 ? segments[segments.length - 1].end : 0,
        sampleRateHz: 16000,
        channels: 1,
        codec: "whisper-wasm",
      },
    };
  }
}
