import test from "node:test";
import assert from "node:assert/strict";
import {
  createRendererBridge,
  createTurnRequest,
  normalizePiiClass,
  readTokenizedChunks,
  tokenizedTextToDocument,
} from "./tauri-bridge.js";

test("provider requests use the exact Tauri command and DTO wire shape", async () => {
  const calls = [];
  const bridge = createRendererBridge((...args) => {
    calls.push(args);
    return Promise.resolve({
      conversation_id: "conversation-1",
      context_id: "context-1",
      chunks: [],
    });
  });
  const request = createTurnRequest({
    conversationId: "conversation-1",
    contextId: "context-1",
    prompt: "Hello",
  });

  await bridge.stream("claude", request);
  assert.deepEqual(calls[0], ["stream-claude", { request }]);
  assert.deepEqual(Object.keys(request).sort(), [
    "context",
    "context_id",
    "conversation_id",
    "prompt",
  ]);
  assert.deepEqual(request.context, []);
});

test("provider switch uses stream-codex and keeps the same conversation IDs", async () => {
  let call;
  const bridge = createRendererBridge((...args) => {
    call = args;
    return Promise.resolve({
      conversation_id: "conversation-2",
      context_id: "context-2",
      chunks: [],
    });
  });
  const request = createTurnRequest({
    conversationId: "conversation-2",
    contextId: "context-2",
    prompt: "Next",
  });

  await bridge.stream("codex", request);
  assert.deepEqual(call, ["stream-codex", { request }]);
});

test("later turns carry prior messages in the Tauri context DTO", () => {
  const context = [
    { label: "User turn 1", content: "Please summarize [PERSON_1]." },
    { label: "Assistant turn 1", content: "[PERSON_1] is mentioned in the note." },
  ];
  assert.deepEqual(createTurnRequest({
    conversationId: "conversation-3",
    contextId: "context-3",
    prompt: "What did they ask?",
    context,
  }), {
    conversation_id: "conversation-3",
    context_id: "context-3",
    prompt: "What did they ask?",
    context,
  });
});

test("open-original forwards opaque reference names without a reveal payload", async () => {
  let call;
  const bridge = createRendererBridge((...args) => {
    call = args;
    return Promise.resolve({ opened: true });
  });
  await bridge.openOriginal({
    documentRef: "doc-ref",
    versionRef: "version-ref",
    originalRef: "original-ref",
    viewerRef: "viewer-ref",
    capabilityRef: "capability-ref",
  });

  assert.deepEqual(call, ["open-original", {
    request: {
      document_ref: "doc-ref",
      version_ref: "version-ref",
      original_ref: "original-ref",
      viewer_ref: "viewer-ref",
      capability_ref: "capability-ref",
    },
  }]);
});

test("malformed provider responses fail closed before any chunks are rendered", () => {
  assert.deepEqual(readTokenizedChunks({
    conversation_id: "c",
    context_id: "x",
    chunks: [{
      conversation_id: "c",
      context_id: "x",
      sequence: 0,
      content: { text: "Bonjour [PERSON_1]", references: [{ token: "[PERSON_1]", class: "person" }] },
    }],
  }, "c", "x"), [{
    sequence: 0,
    text: "Bonjour [PERSON_1]",
    references: [{ token: "[PERSON_1]", class: "person" }],
  }]);

  assert.throws(() => readTokenizedChunks({
    conversation_id: "other",
    context_id: "x",
    chunks: [{ content: { text: "plaintext", references: [] } }],
  }, "c", "x"), /Invalid tokenized response/);
});

test("PII category names from the Rust DTO retain meaningful classes", () => {
  assert.equal(normalizePiiClass("PhoneNumber"), "phone");
  assert.equal(normalizePiiClass("FullName"), "person");
  assert.equal(normalizePiiClass("CreditCard"), "financial");
  assert.equal(normalizePiiClass("unknown-private-value"), "sensitive");
});

test("tokenized response content becomes canonical Tiptap JSON with sensitive marks", () => {
  assert.deepEqual(tokenizedTextToDocument({
    text: "Bonjour [PERSON_1]\nContact [EMAIL_1]",
    references: [
      { token: "[PERSON_1]", class: "Person" },
      { token: "[EMAIL_1]", class: "Email" },
    ],
  }), {
    type: "doc",
    content: [
      {
        type: "paragraph",
        content: [
          { type: "text", text: "Bonjour " },
          { type: "text", text: "[PERSON_1]", marks: [{ type: "haciendaSensitive", attrs: {
            tokenId: "[PERSON_1]", tokenClass: "Person", mode: "pseudonymize", source: "provider", disclosure: "viewer-only",
          } }] },
        ],
      },
      {
        type: "paragraph",
        content: [
          { type: "text", text: "Contact " },
          { type: "text", text: "[EMAIL_1]", marks: [{ type: "haciendaSensitive", attrs: {
            tokenId: "[EMAIL_1]", tokenClass: "Email", mode: "pseudonymize", source: "provider", disclosure: "viewer-only",
          } }] },
        ],
      },
    ],
  });
});

test("missing Tauri invoke capability fails closed", async () => {
  const bridge = createRendererBridge(undefined);
  await assert.rejects(bridge.stream("claude", createTurnRequest({
    conversationId: "c",
    contextId: "x",
    prompt: "hello",
  })), /Desktop provider is unavailable/);
});
