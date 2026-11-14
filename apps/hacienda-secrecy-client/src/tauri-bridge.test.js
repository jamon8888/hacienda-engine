import test from "node:test";
import assert from "node:assert/strict";
import {
  createRendererBridge,
  createTurnRequest,
  canSendToProvider,
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

test("document bridge accepts token-only projections and appends editor versions", async () => {
  const calls = [];
  const projection = {
    schema_version: "hacienda.editor.v1",
    document_id: "doc_opaque",
    version_id: "ver_opaque",
    source: {
      mime_type: "application/pdf",
      original_ref: "orig_opaque",
      viewer_ref: "viewer_opaque",
      capability_ref: "cap_opaque",
    },
    metadata_token_only: { title: [] },
    editor: { type: "doc", content: [{ type: "paragraph", content: [{ type: "text", text: "[PERSON:k1:ABC]" }] }] },
    search_projection: { tokens: [] },
    provenance: { offsets_are_source_only: true },
  };
  const bridge = createRendererBridge(async (...args) => {
    calls.push(args);
    if (args[0] === "pick-document" || args[0] === "load-document") return projection;
    return { document_id: "doc_opaque", version_id: "ver_next", parent_version_id: "ver_opaque", idempotent: false };
  });
  assert.equal((await bridge.pickDocument()).source.original_ref, "orig_opaque");
  await bridge.saveDocument({ documentRef: "doc_opaque", versionRef: "ver_opaque", editor: projection.editor });
  assert.deepEqual(calls[0], ["pick-document"]);
  assert.deepEqual(calls[1], ["save-document", {
    request: { document_ref: "doc_opaque", version_ref: "ver_opaque", editor: projection.editor },
  }]);
  await assert.rejects(
    bridge.saveDocument({ documentRef: "/private/path", versionRef: "ver_opaque", editor: projection.editor }),
    /Invalid document save request/,
  );
});

test("local viewer opens only with validated tokenized DTOs and opaque references", async () => {
  let call;
  const bridge = createRendererBridge(async (...args) => {
    call = args;
    return { opened: true };
  });
  const tokenized = {
    text: "[PERSON:session:opaque]",
    references: [{ token: "[PERSON:session:opaque]", class: "PERSON" }],
  };
  await bridge.openLocalViewer(tokenized, tokenized.references);
  assert.deepEqual(call, ["open-local-viewer", { request: { tokenized, references: tokenized.references } }]);
  await assert.rejects(bridge.openLocalViewer(tokenized, []), /Invalid tokenized response/);
});

test("local viewer reads token-only DTOs and reveals only on explicit token request", async () => {
  const calls = [];
  const bridge = createRendererBridge(async (...args) => {
    calls.push(args);
    if (args[0] === "local-viewer-content") {
      return {
        tokenized: { text: "[PERSON:session:opaque]", references: [{ token: "[PERSON:session:opaque]", class: "PERSON" }] },
        references: [{ token: "[PERSON:session:opaque]", class: "PERSON" }],
      };
    }
    return "private value";
  });
  assert.deepEqual(await bridge.localViewerContent(), {
    tokenized: { text: "[PERSON:session:opaque]", references: [{ token: "[PERSON:session:opaque]", class: "PERSON" }] },
    references: [{ token: "[PERSON:session:opaque]", class: "PERSON" }],
  });
  assert.equal(await bridge.revealLocalToken("[PERSON:session:opaque]"), "private value");
  assert.deepEqual(calls, [
    ["local-viewer-content"],
    ["reveal-local-token", { token: "[PERSON:session:opaque]" }],
  ]);

  const malformedViewer = createRendererBridge(async () => ({
    tokenized: { text: "[PERSON:opaque]", references: [{ token: "[PERSON:opaque]", class: "PERSON" }] },
    references: [{ token: "", class: "PERSON" }],
  }));
  await assert.rejects(malformedViewer.localViewerContent(), /Invalid tokenized response/);
});

test("manual pseudonymization uses the Tauri request and accepts only one returned token", async () => {
  let call;
  const bridge = createRendererBridge(async (...args) => {
    call = args;
    return { text: "[PERSON:session:opaque]", references: [{ token: "[PERSON:session:opaque]", class: "PERSON" }] };
  });
  const tokenized = await bridge.pseudonymizeSelection({ category: "person", value: "private value" });
  assert.deepEqual(call, ["pseudonymize-selection", { request: { category: "person", value: "private value" } }]);
  assert.equal(tokenized.text, tokenized.references[0].token);

  const malformed = createRendererBridge(async () => ({
    text: "plaintext",
    references: [{ token: "[PERSON:session:opaque]", class: "PERSON" }],
  }));
  await assert.rejects(malformed.pseudonymizeSelection({ category: "person", value: "private value" }), /Local pseudonymization failed closed/);
});

test("provider send is enabled only by a complete local model status", async () => {
  assert.equal(canSendToProvider({ provider_send_enabled: true, detector: "local_model" }), true);
  assert.equal(canSendToProvider({ provider_send_enabled: false, detector: "local_model" }), false);
  assert.equal(canSendToProvider({ provider_send_enabled: false, detector: "regex_only" }), false);
  assert.equal(canSendToProvider(undefined), false);

  let call;
  const bridge = createRendererBridge(async (...args) => {
    call = args;
    return { provider_send_enabled: false, detector: "regex_only" };
  });
  assert.deepEqual(await bridge.localPiiStatus(), { provider_send_enabled: false, detector: "regex_only" });
  assert.deepEqual(call, ["local-pii-status"]);

  const malformedStatus = createRendererBridge(async () => ({ provider_send_enabled: "true", detector: "local_model" }));
  await assert.rejects(malformedStatus.localPiiStatus(), /Local PII status unavailable/);
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

test("empty or malformed token references are rejected at the bridge boundary", async () => {
  const malformedReferences = [
    { token: "", class: "person" },
    { token: "   ", class: "person" },
    { token: "[PERSON_1]", class: "" },
    { token: "[PERSON_1]", class: "  " },
    { class: "person" },
    { token: "[PERSON_1]" },
  ];

  for (const reference of malformedReferences) {
    const bridge = createRendererBridge(async () => ({
      conversation_id: "c",
      context_id: "x",
      chunks: [{
        conversation_id: "c",
        context_id: "x",
        sequence: 0,
        content: { text: "Private response", references: [reference] },
      }],
    }));
    await assert.rejects(
      bridge.stream("claude", createTurnRequest({ conversationId: "c", contextId: "x", prompt: "hello" })),
      (error) => error.message === "Invalid tokenized response" && !error.message.includes("Private response"),
    );
  }
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
            tokenId: "[PERSON_1]", tokenClass: "Person", mode: "pseudonymize", source: "regex", disclosure: "viewer-only",
          } }] },
        ],
      },
      {
        type: "paragraph",
        content: [
          { type: "text", text: "Contact " },
          { type: "text", text: "[EMAIL_1]", marks: [{ type: "haciendaSensitive", attrs: {
            tokenId: "[EMAIL_1]", tokenClass: "Email", mode: "pseudonymize", source: "regex", disclosure: "viewer-only",
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
