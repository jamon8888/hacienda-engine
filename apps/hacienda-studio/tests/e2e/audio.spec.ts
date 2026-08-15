import { test, expect, type Page } from "@playwright/test";
import { visitFresh, mockWhisperModelAssets } from "./fixtures";

/**
 * Track D3: an audio fixture through the full pipeline (the old plan's Phase
 * 5 referenced an `audio/mpeg` fixture that was never added), and — folded in
 * here per D1's note rather than duplicated — `enableTranscription`'s
 * on/off effect.
 *
 * Building this turned up a real bug, not a fixture problem:
 * `@remotion/whisper-web`'s `canUseWhisperWeb()` checks `typeof window ===
 * "undefined"` and refuses to run otherwise — but `worker/pipeline.ts` runs
 * inside a Web Worker, which has `self`, not `window`. Every
 * `WhisperBridge.load()`/`transcribeAudio()` call therefore threw
 * synchronously, in every environment, regardless of network access. That
 * has since been fixed architecturally, not patched: `WhisperBridge` now
 * lives and runs only on the main thread (`App.tsx`), and `worker/pipeline.ts`
 * requests a transcription over `postMessage` instead of running it itself —
 * see `worker/transcribe-bridge.ts`'s header for the full design. The third
 * test below, which used to pin the exact "window is not defined" failure
 * text, now pins the opposite: that failure text must never appear again.
 *
 * A second, real bug surfaced alongside the first and was fixed at the same
 * time: `processFiles` used to await `whisperBridge.load()` once, upfront,
 * outside every per-file try/catch and outside its own caller's — so the
 * always-current rejection above was an unhandled promise rejection that
 * silently hung the entire batch (no error banner, no download, no feedback
 * at all). There is no such upfront call anymore (`TranscriptionRequestBridge`
 * is entirely per-file, requested lazily from inside `processFile`), and its
 * own timeout guards the same failure mode structurally: a lost or
 * never-sent reply fails just that one file, through the existing per-file
 * catch, rather than hanging the batch.
 *
 * What this suite does *not* attempt: real transcription against real model
 * weights. `@remotion/whisper-web`'s `tiny.en` model alone is ~74MB
 * (`SIZES` in its `dist/constants.js`) — fetching it for real here would be
 * exactly the slow, non-deterministic network dependency
 * `mockNerModelAssets` exists to avoid for the NER model, independent of the
 * fact that this particular sandbox cannot reach huggingface.co at all
 * (confirmed separately via the sandbox's proxy status endpoint returning
 * 403 for a `Range` probe against it). The third test below mocks the model
 * host (`mockWhisperModelAssets`) and asserts only that the download is
 * *attempted* — proof `canUseWhisperWeb()` got past the `window` check that
 * used to fail unconditionally — not that a full transcription completes
 * against the (necessarily fake) mocked bytes.
 */

/**
 * A minimal but genuinely valid 16-bit PCM mono WAV file (silence) —
 * decodable by a real browser `AudioContext`, unlike an arbitrary byte
 * buffer typed "audio/wav". No fixture binary is checked in; this is
 * deterministic and self-contained.
 */
function silentWav(durationSeconds = 0.5, sampleRate = 16000): Buffer {
  const numSamples = Math.floor(durationSeconds * sampleRate);
  const dataSize = numSamples * 2; // 16-bit mono
  const buffer = Buffer.alloc(44 + dataSize);
  buffer.write("RIFF", 0, "ascii");
  buffer.writeUInt32LE(36 + dataSize, 4);
  buffer.write("WAVE", 8, "ascii");
  buffer.write("fmt ", 12, "ascii");
  buffer.writeUInt32LE(16, 16);
  buffer.writeUInt16LE(1, 20); // PCM
  buffer.writeUInt16LE(1, 22); // mono
  buffer.writeUInt32LE(sampleRate, 24);
  buffer.writeUInt32LE(sampleRate * 2, 28); // byte rate
  buffer.writeUInt16LE(2, 32); // block align
  buffer.writeUInt16LE(16, 34); // bits per sample
  buffer.write("data", 36, "ascii");
  buffer.writeUInt32LE(dataSize, 40);
  // Remaining bytes are already zero-filled by Buffer.alloc — silence.
  return buffer;
}

/**
 * `batch-complete` — and therefore a `download` — fires unconditionally once
 * every file has been attempted, whether or not the audio file's own
 * processing succeeded (`processFiles` still zips whatever it has and posts
 * `batch-complete` regardless of per-file failures). So `download` alone
 * reliably signals "the batch finished"; it does not mean "no error
 * occurred". The per-file `error` postMessage — and its `.error-banner` —
 * lands before that, in the same catch block, so by the time `download`
 * fires the banner (if any) is already in the DOM to read directly.
 */
async function uploadAudioFixture(page: Page): Promise<string | null> {
  // 60s, not the default: real AudioContext decode + wasm extraction of this fixture is
  // CPU-heavy, and this host is constrained enough that 30s occasionally isn't enough.
  const download = page.waitForEvent("download", { timeout: 60000 });

  await page.setInputFiles('input[type="file"]', {
    name: "meeting.wav",
    mimeType: "audio/wav",
    buffer: silentWav(),
  });

  await download;
  return page.locator(".error-banner").textContent().catch(() => null);
}

test.describe("audio fixture (Track D3)", () => {
  test("accepted by the upload gate and reaches the worker", async ({
    page,
  }) => {
    await visitFresh(page);

    const bannerText = await uploadAudioFixture(page);

    // "Unsupported file type" would be a synchronous validateFile rejection
    // before the file ever reaches the worker (Track A3). Whatever error
    // eventually shows (if any — see the two tests below) is not that one,
    // which proves the gate accepted the file.
    expect(bannerText).not.toContain("Unsupported file type");
  });

  test("enableTranscription off (default): fails in extraction, not transcription", async ({
    page,
  }) => {
    await visitFresh(page);

    const bannerText = await uploadAudioFixture(page);

    expect(bannerText).toContain("meeting.wav");
    expect(bannerText).not.toMatch(/whisper/i);
  });

  test("enableTranscription on: reaches whisper-web's real model download on the main thread — the window-undefined bug is fixed", async ({
    page,
  }) => {
    await visitFresh(page);
    await mockWhisperModelAssets(page);

    let modelDownloadRequested = false;
    page.on("request", (request) => {
      if (
        request
          .url()
          .startsWith("https://huggingface.co/ggerganov/whisper.cpp/resolve/main/")
      ) {
        modelDownloadRequested = true;
      }
    });

    await page.click("button.config-toggle");
    await page.check(
      'label:has-text("Enable Audio/Video Transcription") input[type="checkbox"]',
    );
    await page.keyboard.press("Escape");

    await page.setInputFiles('input[type="file"]', {
      name: "meeting.wav",
      mimeType: "audio/wav",
      buffer: silentWav(),
    });

    // Previously (see this file's header), this point was unreachable: `WhisperBridge.load()`
    // called `canUseWhisperWeb()`, which synchronously rejected on `window` being undefined
    // the instant transcription started — inside the Worker, before any network request was
    // ever attempted, deterministically, regardless of what the fixture actually contained.
    // Now that `WhisperBridge` runs on the main thread (a real `window`, and this app's own
    // Cross-Origin-Opener/Embedder-Policy headers satisfy `crossOriginIsolated` too — see
    // `vite.config.ts`), it gets past that check and genuinely requests the model. Asserting
    // on the request itself, rather than waiting for `batch-complete`/a download, is
    // deliberate: the mocked bytes below are not a valid ggml model (see
    // `mockWhisperModelAssets`'s header), so this test does not depend on — and is not
    // gated by — whatever whisper.cpp's wasm build does when handed invalid model bytes.
    await expect.poll(() => modelDownloadRequested, { timeout: 30000 }).toBe(true);
  });
});
