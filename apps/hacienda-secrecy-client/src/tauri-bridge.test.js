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

test("local hybrid search sends the query only to the local Tauri command and validates token-only results", async () => {
  const calls = [];
  const bridge = createRendererBridge(async (...args) => {
    calls.push(args);
    return { results: [{
      title_token_only: "Dossier [PERSON:k1:ABC]",
      content_token_only: "Facture [EMAIL:k1:DEF]",
      ordinal: 2,
      score: 0.81,
    }] };
  });
  const results = await bridge.searchLocalDocuments("adresse de [PERSON:k1:ABC]");
  assert.deepEqual(calls, [["search-local-documents", { request: { query: "adresse de [PERSON:k1:ABC]" } }]]);
  assert.equal(results[0].ordinal, 2);
  assert.equal(results[0].score, 0.81);
  await assert.rejects(bridge.searchLocalDocuments(" "), /empty or too long/);
  await assert.rejects(bridge.searchLocalDocuments("x".repeat(16_001)), /empty or too long/);
});

test("local hybrid search rejects malformed or oversized responses", async () => {
  for (const response of [
    { results: [{ title_token_only: "", content_token_only: "snippet", ordinal: 0, score: 0.1 }] },
    { results: [{ title_token_only: "title", content_token_only: "snippet", ordinal: -1, score: 0.1 }] },
    { results: [{ title_token_only: "title", content_token_only: "snippet", ordinal: 0, score: Number.NaN }] },
    { results: Array.from({ length: 9 }, () => ({ title_token_only: "title", content_token_only: "snippet", ordinal: 0, score: 0.1 })) },
  ]) {
    const bridge = createRendererBridge(async () => response);
    await assert.rejects(bridge.searchLocalDocuments("query"), /Invalid local search response/);
  }
});

test("local RAG activation accepts only the pinned multilingual preset", async () => {
  const bridge = createRendererBridge(async (command) => {
    assert.equal(command, "install-local-rag-model");
    return { ready: true, model: "xberg:preset:multilingual" };
  });
  assert.deepEqual(await bridge.installLocalRagModel(), {
    ready: true,
    model: "xberg:preset:multilingual",
  });

  const malformed = createRendererBridge(async () => ({ ready: true, model: "arbitrary-model" }));
  await assert.rejects(malformed.installLocalRagModel(), /retrieval model is unavailable/);
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

test("TeamOnPremise ingest forwards only opaque refs and never a path, token or raw text", async () => {
  const calls = [];
  const bridge = createRendererBridge(async (...args) => {
    calls.push(args);
    return { job_id: "job-opaque-1" };
  });

  const accepted = await bridge.teamEmailIngest({
    sourceRef: "default",
    mailboxRef: "inbox",
    operation: "index_for_rag",
    privacyMode: "redact",
    // Extra attacker-controlled keys must be dropped by the allowlist payload.
    backend: { path: "/etc" },
    token: "oauth-secret",
    password: "hunter2",
    token_command: ["sh"],
    message: "jean.dupont@example.com",
  });
  assert.deepEqual(accepted, { job_id: "job-opaque-1" });
  assert.deepEqual(calls, [["team-email-ingest", {
    request: {
      source_ref: "default",
      mailbox_ref: "inbox",
      operation: { operation: "index_for_rag" },
      privacy_mode: "redact",
    },
  }]]);
  assert.ok(!JSON.stringify(calls).includes("/etc"));
  assert.ok(!JSON.stringify(calls).includes("oauth-secret"));
  assert.ok(!JSON.stringify(calls).includes("jean.dupont@example.com"));

  for (const request of [
    { sourceRef: "/etc", mailboxRef: "inbox", operation: "index_for_rag" },
    { sourceRef: "default", mailboxRef: "../victim", operation: "index_for_rag" },
    { sourceRef: "default", mailboxRef: "inbox", operation: "delete_everything" },
    { sourceRef: "default", mailboxRef: "inbox", operation: "redact", privacyMode: "plaintext" },
    // Deterministic tokens are a PII oracle on the team surface.
    { sourceRef: "default", mailboxRef: "inbox", operation: "redact", privacyMode: "pseudonymize" },
  ]) {
    await assert.rejects(bridge.teamEmailIngest(request), /Invalid team/);
  }
});

test("TeamOnPremise viewer sends only the capability reference", async () => {
  const calls = [];
  const bridge = createRendererBridge(async (...args) => {
    calls.push(args);
    if (args[0] === "team-viewer-capability") {
      return {
        capability_ref: "vcap_opaque",
        expires_at: "2026-11-21T12:05:00Z",
        document_ref: "ref_doc",
        email_ref: "ref_email",
        attachment_ref: null,
        delivery: "server_side",
      };
    }
    if (args[0] === "team-viewer-reveal") {
      return { capability_ref: "vcap_opaque", revealed: true, document_ref: "ref_doc", email_ref: "ref_email" };
    }
    return { events: [] };
  });

  await bridge.teamViewerCapability({
    documentRef: "ref_doc",
    emailRef: "ref_email",
    tokenRef: "[EMAIL:k1:ABC]",
    value: "jean.dupont@example.com",
    path: "/srv/mail",
  });
  assert.deepEqual(calls[0], ["team-viewer-capability", {
    request: { document_ref: "ref_doc", email_ref: "ref_email", token_ref: "[EMAIL:k1:ABC]" },
  }]);

  await bridge.teamViewerReveal({
    capabilityRef: "vcap_opaque",
    plaintext: "jean.dupont@example.com",
    path: "/srv/mail/inbox",
  });
  assert.deepEqual(calls[1], ["team-viewer-reveal", { request: { capability_ref: "vcap_opaque" } }]);
  assert.ok(!JSON.stringify(calls).includes("jean.dupont@example.com"));
  assert.deepEqual(await bridge.teamViewerAudit(), { events: [] });
});

test("TeamOnPremise viewer open returns a typed, content-safe projection", async () => {
  const projection = {
    capability_ref: "vcap_opaque",
    document_ref: "ref_doc",
    email_ref: "ref_email",
    headers: [
      { name: "subject", token_only: "[REDACTED] [EMAIL:k1:ABC]" },
      { name: "from", token_only: "[REDACTED]" },
    ],
    body_token_only: "Bonjour [PERSON:k1:DEF]",
    attachment_refs: ["ref_attach"],
    projection_kind: "redacted_token_only",
  };
  const calls = [];
  const bridge = createRendererBridge(async (...args) => {
    calls.push(args);
    return projection;
  });

  assert.deepEqual(
    await bridge.teamViewerOpen({ capabilityRef: "vcap_opaque", plaintext: "jean.dupont@example.com" }),
    projection,
  );
  assert.deepEqual(calls, [["team-viewer-open", { request: { capability_ref: "vcap_opaque" } }]]);
  assert.ok(!JSON.stringify(calls).includes("jean.dupont@example.com"));

  // A boolean-only `opened:true` stub is no longer a valid projection.
  const legacy = createRendererBridge(async () => ({ opened: true }));
  await assert.rejects(legacy.teamViewerOpen({ capabilityRef: "vcap_opaque" }), /Invalid team viewer open projection/);
});

test("TeamOnPremise renderer rejects DTOs carrying a content or path carrier", async () => {
  const badSources = createRendererBridge(async () => ({
    sources: [{ source_ref: "default", mailbox_refs: ["inbox"], path: "/srv/mail" }],
  }));
  await assert.rejects(badSources.teamEmailSources(), /Invalid team email sources response/);

  const badJob = createRendererBridge(async () => ({
    job_id: "job-opaque", status: "succeeded", failed: false,
    result: { plaintext: "jean.dupont@example.com" }, progress: null,
  }));
  await assert.rejects(badJob.teamEmailJob({ jobId: "job-opaque" }), /Invalid team email job status/);

  const badReveal = createRendererBridge(async () => ({
    capability_ref: "vcap", revealed: true, document_ref: "d", email_ref: "e",
    value: "jean.dupont@example.com",
  }));
  await assert.rejects(
    badReveal.teamViewerReveal({ capabilityRef: "vcap" }),
    /Invalid team viewer reveal result/,
  );
});

test("TeamOnPremise async job results are projected identically for polling", async () => {
  const result = {
    schema_version: 1, success: true, processed_count: 2, entities_found: 1,
    error_count: 0, processing_time_ms: 12,
    redaction_count: 3, redaction_by_type: { email: 3 },
    chunks_created: 4, vectors_indexed: 4,
  };
  const calls = [];
  const bridge = createRendererBridge(async (...args) => {
    calls.push(args);
    return {
      job_id: "job-opaque", status: "succeeded", failed: false,
      progress: null, result,
    };
  });
  const viaJob = await bridge.teamEmailJob({ jobId: "job-opaque" });
  const viaResult = await bridge.teamEmailJobResult({ jobId: "job-opaque" });
  assert.deepEqual(viaJob.result, result);
  assert.deepEqual(viaResult.result, result);
  assert.deepEqual(calls.map((call) => call[0]), ["team-email-job", "team-email-job-result"]);
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
