/* Faithful simulation of the AI SDK's processUIMessageStream (from the
 * verified source in ai/dist/index.js, lines ~6996-7010 and ~7377-7385):
 *
 *   case "text-start":  push a text part, store in activeTextParts[chunk.id]
 *   case "text-delta":  activeTextParts[chunk.id].text += chunk.delta
 *   case "text-end":    (finalize)
 *   case "finish":      state.finishReason = chunk.finishReason
 *
 * It fetches the adapter stream (via the Vite proxy) and runs this exact
 * logic to prove the browser assembles the harness reply correctly.
 */
const URL = process.env.CHAT_URL ?? "http://localhost:5174/api/chat";
const PROMPT = "Reply with exactly: assembled-ok";

async function main() {
  const res = await fetch(URL, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ messages: [{ role: "user", parts: [{ type: "text", text: PROMPT }] }] }),
  });
  if (!res.ok || !res.body) throw new Error("HTTP " + res.status);

  const reader = res.body.getReader();
  const decoder = new TextDecoder();
  let buffer = "";
  const activeText = new Map();
  const textParts = [];
  let finishReason = null;
  let sawDone = false;

  while (true) {
    const { done, value } = await reader.read();
    if (done) break;
    buffer += decoder.decode(value, { stream: true });
    const lines = buffer.split("\n");
    buffer = lines.pop() ?? "";
    for (const raw of lines) {
      const line = raw.trim();
      if (!line.startsWith("data:")) continue;
      const data = line.slice(5).trim();
      if (data === "[DONE]") { sawDone = true; continue; }
      const chunk = JSON.parse(data);
      switch (chunk.type) {
        case "text-start":
          textParts.push({ id: chunk.id, text: "" });
          activeText.set(chunk.id, textParts[textParts.length - 1]);
          break;
        case "text-delta": {
          const p = activeText.get(chunk.id);
          if (!p) throw new Error(`text-delta for missing part ${chunk.id} (exactly what the browser throws)`);
          p.text += chunk.delta;
          break;
        }
        case "text-end":
          break;
        case "finish":
          finishReason = chunk.finishReason;
          break;
        default:
          throw new Error("unknown chunk type: " + chunk.type);
      }
    }
  }

  const joined = textParts.map((p) => p.text).join("");
  console.log("finishReason:", finishReason);
  console.log("assembled text:", JSON.stringify(joined));
  console.log("saw [DONE]:", sawDone);

  if (joined.includes("assembled-ok") && finishReason === "stop" && sawDone) {
    console.log("\nSUCCESS: browser message state machine assembles the harness reply.");
    process.exit(0);
  }
  console.error("\nFAILURE.");
  process.exit(2);
}

main().catch((e) => {
  console.error(e);
  process.exit(2);
});
