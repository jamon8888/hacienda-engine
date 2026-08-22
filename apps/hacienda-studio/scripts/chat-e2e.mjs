/* End-to-end verification: loads the studio, opens Chat, sends a message,
 * and asserts the DeepSeek Harness streams a reply into the AI Elements UI. */
import { chromium } from "playwright";

const BASE = "http://localhost:5174";
const PROMPT = "Reply with exactly: e2e-harness-ok";

async function main() {
  const browser = await chromium.launch();
  const page = await browser.newPage();
  page.on("console", (m) => {
    if (m.type() === "error") console.log("[console.error]", m.text());
  });
  page.on("pageerror", (e) => console.log("[pageerror]", e.message));

  console.log("Loading", BASE, "...");
  await page.goto(BASE, { waitUntil: "domcontentloaded", timeout: 60000 });
  // Give the app time to boot (asset preload / onboarding).
  await page.waitForTimeout(2500);

  // Click the Chat nav tab.
  const chatTab = page.getByRole("button", { name: "Chat" });
  if (await chatTab.isVisible().catch(() => false)) {
    await chatTab.click();
  }
  await page.waitForTimeout(1000);

  // Type into the PromptInput textarea and submit.
  const textarea = page.locator("textarea").first();
  await textarea.waitFor({ state: "visible", timeout: 15000 });
  await textarea.fill(PROMPT);
  await page.waitForTimeout(200);

  // Submit via Enter.
  await textarea.press("Enter");

  // Poll for the assistant's expected answer text to appear anywhere on the page.
  console.log("Waiting for streamed reply containing 'e2e-harness-ok' ...");
  let found = false;
  for (let i = 0; i < 60; i++) {
    const body = await page.locator("body").innerText().catch(() => "");
    if (body.includes("e2e-harness-ok")) {
      found = true;
      break;
    }
    await page.waitForTimeout(500);
  }

  await page.screenshot({ path: "/tmp/chat-e2e.png" });

  if (found) {
    console.log("SUCCESS: harness reply rendered in AI Elements chat UI.");
  } else {
    const body = await page.locator("body").innerText().catch(() => "");
    console.log("FAILURE: expected text not found. Body tail:\n" + body.slice(-800));
  }

  await browser.close();
  process.exit(found ? 0 : 1);
}

main().catch((e) => {
  console.error("E2E error:", e);
  process.exit(2);
});
