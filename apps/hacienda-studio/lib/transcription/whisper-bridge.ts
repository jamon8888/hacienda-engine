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
  ): Promise<TranscriptionResult> {
    if (!this.loaded) {
      await this.load(config.modelSize || "tiny.en");
    }

    console.log("[WhisperBridge] Resampling audio...");
    const file = new File([audioBytes], "audio", { type: mimeType });
    const channelWaveform = await resampleTo16Khz({
      file,
      onProgress: (p) =>
        console.log(
          `[WhisperBridge] Resampling audio (${Math.round(p * 100)}%)...`,
        ),
    });

    console.log("[WhisperBridge] Transcribing...");
    const { transcription } = await transcribe({
      channelWaveform,
      model: this.modelSize as any,
      onProgress: (p) =>
        console.log(
          `[WhisperBridge] Transcribing (${Math.round(p * 100)}%)...`,
        ),
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
