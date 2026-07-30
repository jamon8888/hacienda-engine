import { test, expect, type Page } from "@playwright/test";

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
 * `WhisperBridge.load()`/`transcribeAudio()` call therefore throws
 * synchronously, in every environment, regardless of network access. This
 * has nothing to do with this sandbox's blocked huggingface.co (confirmed
 * separately via the sandbox's proxy status endpoint) — transcription cannot
 * currently succeed anywhere. Fixing that is an architecture change (running
 * whisper-web on the main thread and bridging results into the worker), not
 * a test-writing task; it is not attempted here.
 *
 * A second, real bug surfaced alongside it and *is* fixed here: `processFiles`
 * awaited `whisperBridge.load()` once, upfront, outside every per-file
 * try/catch and outside its own caller's — so the always-current rejection
 * above was an unhandled promise rejection that silently hung the entire
 * batch (no error banner, no download, no feedback at all). Wrapped in
 * worker/pipeline.ts now, so the failure instead surfaces per-file through
 * the normal error-banner path, same as any other file-processing failure.
 *
 * Given that, these tests assert what's actually true today: the upload gate
 * accepts audio, and the two toggle states produce two different,
 * *specific* error messages (extraction can't handle audio at all vs.
 * whisper-web can't run in a worker) — not the same generic failure, so the
 * toggle still has a provable effect even though neither path succeeds yet.
 */

async function visitFresh(page: Page): Promise<void> {
  await page.addInitScript(() => {
    localStorage.setItem("xberg-studio-visited", "true");
  });
  await page.goto("/");
  await page.waitForSelector('input[type="file"]:not([disabled])');
}

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
  const download = page.waitForEvent("download", { timeout: 30000 });

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

  test("enableTranscription on: fails in whisper-web, not extraction — a different error than off", async ({
    page,
  }) => {
    await visitFresh(page);
    await page.click("button.config-toggle");
    await page.check(
      'label:has-text("Enable Audio/Video Transcription") input[type="checkbox"]',
    );
    await page.keyboard.press("Escape");

    const bannerText = await uploadAudioFixture(page);

    expect(bannerText).toContain("meeting.wav");
    // The known bug this file documents, not a guess: whisper-web requires
    // `window`, which a Worker does not have.
    expect(bannerText).toMatch(/whisper web is not supported/i);
    expect(bannerText).toMatch(/window.*not defined/i);
  });
});
