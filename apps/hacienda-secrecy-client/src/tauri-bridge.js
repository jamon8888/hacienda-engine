// @ts-check

/** @typedef {{ label: string, content: string }} PromptContext */
/** @typedef {{ conversation_id: string, context_id: string, prompt: string, context: PromptContext[] }} ProviderRequest */
/** @typedef {{ token: string, class: string }} OpaqueTokenReference */
/** @typedef {{ text: string, references: OpaqueTokenReference[] }} TokenizedText */
/** @typedef {{ conversation_id: string, context_id: string, sequence: number, content: TokenizedText }} ProviderStreamChunk */
/** @typedef {{ conversation_id: string, context_id: string, chunks: ProviderStreamChunk[] }} ProviderResponse */
/** @typedef {{ document_ref: string, version_ref: string, original_ref: string, viewer_ref: string, capability_ref: string }} OpenOriginalRequest */
/** @typedef {{ documentRef: string, versionRef: string, originalRef: string, viewerRef: string, capabilityRef: string }} OpenOriginalReferences */
/** @typedef {{ tokenized: TokenizedText, references: OpaqueTokenReference[] }} OpenLocalViewerRequest */
/** @typedef {{ provider_send_enabled: boolean, detector: "local_model" | "regex_only" }} LocalPiiStatus */
/** @typedef {{ schema_version: string, document_id: string, version_id: string, source: { mime_type: string, original_ref: string, viewer_ref: string, capability_ref: string }, metadata_token_only: Record<string, unknown>, editor: Record<string, unknown>, search_projection: Record<string, unknown>, provenance: Record<string, unknown> }} DocumentProjection */
/** @typedef {{ source_ref: string, mailbox_ref: string, operation: { operation: string }, privacy_mode?: string, dossier_id?: string }} TeamIngestRequest */
/** @typedef {{ job_id: string }} TeamJobRequest */
/** @typedef {{ source_ref: string, mailbox_ref: string, query_text: string, top_k?: number, dossier_id?: string }} TeamSearchRequest */
/** @typedef {{ document_ref: string, email_ref: string, token_ref: string, attachment_ref?: string, purpose?: string }} TeamViewerCapabilityRequest */
/** @typedef {{ capability_ref: string }} TeamViewerActionRequest */
/** @typedef {{ request: ProviderRequest | OpenOriginalRequest | OpenLocalViewerRequest | { category: string, value: string } | { query: string } | { document_ref: string, version_ref: string } | { document_ref: string, version_ref: string, editor: Record<string, unknown> } | TeamIngestRequest | TeamJobRequest | TeamSearchRequest | TeamViewerCapabilityRequest | TeamViewerActionRequest } | { token: string } | { job_id: string } | { capability_ref: string }} InvokePayload */
/** @typedef {{ title_token_only: string, content_token_only: string, ordinal: number, score: number }} LocalSearchResult */
/** @typedef {(command: "stream-claude" | "stream-codex" | "open-original" | "open-local-viewer" | "local-viewer-content" | "reveal-local-token" | "pseudonymize-selection" | "local-pii-status" | "pick-document" | "install-local-rag-model" | "search-local-documents" | "load-document" | "save-document" | "team-email-sources" | "team-email-ingest" | "team-email-job" | "team-email-job-result" | "team-email-search" | "team-viewer-capability" | "team-viewer-reveal" | "team-viewer-open" | "team-viewer-audit", payload?: InvokePayload) => Promise<unknown>} RendererInvoke */
/** @typedef {{ sequence: number, text: string, references: OpaqueTokenReference[] }} TokenizedChunk */
/** @typedef {{ source_ref: string, mailbox_refs: string[] }} TeamEmailSource */
/** @typedef {{ sources: TeamEmailSource[] }} TeamEmailSourcesResponse */
/** @typedef {{ job_id: string }} TeamAcceptedJob */
/** @typedef {{ job_id: string, status: string, failed: boolean, progress: Record<string, unknown> | null, result: Record<string, unknown> | null }} TeamJobStatus */
/** @typedef {{ document_ref: string, external_ref: string, source_kind: string, mime: string | null, dossier_id: string | null, version_ref: string | null, ordinal: number, score: number, projection: string, viewer_policy: string }} TeamEmailSearchHit */
/** @typedef {{ hits: TeamEmailSearchHit[] }} TeamEmailSearchResponse */
/** @typedef {{ capability_ref: string, expires_at: string, document_ref: string, email_ref: string, attachment_ref: string | null, delivery: string }} TeamViewerCapability */
/** @typedef {{ capability_ref: string, revealed: boolean, document_ref: string, email_ref: string }} TeamViewerRevealResult */
/** @typedef {{ name: string, token_only: string }} TeamViewerHeader */
/** @typedef {{ capability_ref: string, document_ref: string, email_ref: string, headers: TeamViewerHeader[], body_token_only: string, attachment_refs: string[], projection_kind: string }} TeamViewerProjection */
/** @typedef {{ sequence: number, timestamp: string, action: string, tenant: string, principal: string, document_ref: string, email_ref: string, attachment_ref: string | null, capability_ref: string, outcome: string }} TeamAuditEvent */
/** @typedef {{ events: TeamAuditEvent[] }} TeamAuditResponse */

const TEAM_INGEST_OPERATIONS = Object.freeze(["index_for_rag", "extract_ner", "redact", "pipeline"]);
// TeamOnPremise has no safe multi-tenant token/reveal contract; only `redact`
// is allowed. `pseudonymize` is rejected here and by the server.
const TEAM_PRIVACY_MODES = Object.freeze(["redact"]);
// A renderer response DTO must not carry a content/path/credential carrier. The
// word may still appear inside a redacted `projection`, so only *keys* are checked.
const FORBIDDEN_TEAM_DTO_KEYS = Object.freeze([
  "path", "backend", "server", "password", "token", "oauth", "token_command",
  "plaintext", "raw", "raw_text", "message", "body", "subject", "from", "to",
  "attachment_data", "value", "content", "credentials",
]);

const PROVIDER_COMMANDS = Object.freeze({
  claude: "stream-claude",
  codex: "stream-codex",
});

const unavailable = () => new Error("Desktop provider is unavailable");
const invalidTokenizedResponse = () => new Error("Invalid tokenized response");

/** @param {unknown} value @returns {value is Record<string, unknown>} */
function isRecord(value) {
  return Boolean(value && typeof value === "object" && !Array.isArray(value));
}

const PII_CLASS_ALIASES = Object.freeze({
  person: "person",
  fullname: "person",
  firstname: "person",
  middlename: "person",
  lastname: "person",
  organization: "organization",
  email: "email",
  phone: "phone",
  phonenumber: "phone",
  matter: "matter",
  location: "location",
  city: "location",
  stateorregion: "location",
  country: "location",
  address: "address",
  streetaddress: "address",
  postalcode: "address",
  date: "date",
  dateofbirth: "date",
  cardexpiry: "date",
  ssn: "identity",
  passportnumber: "identity",
  driverslicense: "identity",
  nationalid: "identity",
  taxid: "identity",
  governmentid: "identity",
  euvat: "identity",
  licenseplate: "identity",
  vehiclevin: "identity",
  creditcard: "financial",
  paymentcard: "financial",
  cardcvv: "financial",
  iban: "financial",
  bankaccount: "financial",
  routingnumber: "financial",
  swiftbic: "financial",
  cryptowallet: "financial",
  medicalrecordnumber: "health",
  healthplannumber: "health",
  diagnosis: "health",
  medication: "health",
  username: "credential",
  password: "credential",
  apikey: "credential",
  secrettoken: "credential",
  jwttoken: "credential",
  ipaddress: "network",
  macaddress: "network",
  url: "network",
});

/** @param {string} value */
export function normalizePiiClass(value) {
  if (typeof value !== "string") return "sensitive";
  const key = value
    .replace(/([a-z0-9])([A-Z])/g, "$1_$2")
    .toLowerCase()
    .replace(/[^a-z0-9]/g, "");
  return /** @type {Record<string, string>} */ (PII_CLASS_ALIASES)[key] ?? "sensitive";
}

/**
 * @param {{ conversationId: string, contextId: string, prompt: string, context?: PromptContext[] }} input
 * @returns {ProviderRequest}
 */
export function createTurnRequest({ conversationId, contextId, prompt, context = [] }) {
  return {
    conversation_id: conversationId,
    context_id: contextId,
    prompt,
    context,
  };
}

/** @param {unknown} value @returns {value is TokenizedText} */
function isTokenizedText(value) {
  if (!isRecord(value) || typeof value.text !== "string" || !Array.isArray(value.references)) {
    return false;
  }
  return value.references.every(
      (reference) =>
        isRecord(reference) &&
        typeof reference.token === "string" &&
        reference.token.trim().length > 0 &&
        typeof reference.class === "string" &&
        reference.class.trim().length > 0,
  );
}

/** @param {unknown} value @returns {value is OpaqueTokenReference[]} */
function isOpaqueTokenReferences(value) {
  return Array.isArray(value) && value.every(
    (reference) =>
      isRecord(reference) &&
      typeof reference.token === "string" &&
      reference.token.trim().length > 0 &&
      typeof reference.class === "string" &&
      reference.class.trim().length > 0,
  );
}

/** @param {unknown} value @returns {value is string} */
function isOpaqueReference(value) {
  return typeof value === "string" && value.trim().length > 0 &&
    !value.includes("/") && !value.includes("\\") && !value.includes("file:");
}

/** @param {unknown} value @returns {value is DocumentProjection} */
function isDocumentProjection(value) {
  if (!isRecord(value) || typeof value.schema_version !== "string" ||
      !isOpaqueReference(value.document_id) || !isOpaqueReference(value.version_id) ||
      !isRecord(value.source) || typeof value.source.mime_type !== "string" ||
      !isOpaqueReference(value.source.original_ref) || !isOpaqueReference(value.source.viewer_ref) ||
      !isOpaqueReference(value.source.capability_ref) ||
      !isRecord(value.metadata_token_only) || !isRecord(value.editor) ||
      value.editor.type !== "doc" || !isRecord(value.search_projection) || !isRecord(value.provenance)) {
    return false;
  }
  return true;
}

/** @param {unknown} response @returns {DocumentProjection} */
function readDocumentProjection(response) {
  if (!isDocumentProjection(response)) throw new Error("Invalid token-only document projection");
  const projection = /** @type {DocumentProjection} */ (response);
  const editor = JSON.parse(JSON.stringify(projection.editor));
  /** @param {any} node */
  const normalize = (node) => {
    if (!node || typeof node !== "object") return;
    if (Array.isArray(node.marks)) {
      node.marks = node.marks.map(/** @param {any} mark */ (mark) => mark?.type === "sensitive"
        ? { ...mark, type: "haciendaSensitive", attrs: {
          tokenClass: mark.attrs?.tokenClass ?? "sensitive",
          tokenId: mark.attrs?.tokenId ?? "manual",
          mode: mark.attrs?.mode ?? "pseudonymize",
          source: mark.attrs?.source ?? "hacienda",
          disclosure: mark.attrs?.disclosureState ?? "viewer-only",
        } }
        : mark);
    }
    if (Array.isArray(node.content)) node.content.forEach(normalize);
  };
  normalize(editor);
  return { ...projection, editor };
}

/**
 * @param {unknown} response
 * @param {string} conversationId
 * @param {string} contextId
 * @returns {asserts response is ProviderResponse}
 */
function assertTokenizedResponse(response, conversationId, contextId) {
  if (
    !isRecord(response) ||
    response.conversation_id !== conversationId ||
    response.context_id !== contextId ||
    !Array.isArray(response.chunks)
  ) {
    throw invalidTokenizedResponse();
  }

  for (const chunk of response.chunks) {
    if (
      !isRecord(chunk) ||
      chunk.conversation_id !== conversationId ||
      chunk.context_id !== contextId ||
      !Number.isSafeInteger(chunk.sequence) ||
      !isTokenizedText(chunk.content)
    ) {
      throw invalidTokenizedResponse();
    }
  }
}

/** @param {TokenizedText} tokenized @param {OpaqueTokenReference[]} references */
function assertLocalViewerPayload(tokenized, references) {
  if (!isTokenizedText(tokenized) || !Array.isArray(references)) {
    throw invalidTokenizedResponse();
  }
  const expected = new Set(tokenized.references.map((reference) => `${reference.token}\u0000${reference.class}`));
  const supplied = new Set();
  for (const reference of references) {
    if (
      !reference ||
      typeof reference.token !== "string" ||
      reference.token.trim().length === 0 ||
      typeof reference.class !== "string" ||
      reference.class.trim().length === 0 ||
      !tokenized.text.includes(reference.token)
    ) {
      throw invalidTokenizedResponse();
    }
    supplied.add(`${reference.token}\u0000${reference.class}`);
  }
  if (expected.size !== tokenized.references.length || supplied.size !== references.length ||
      expected.size !== supplied.size || [...expected].some((reference) => !supplied.has(reference))) {
    throw invalidTokenizedResponse();
  }
}

/** @param {unknown} response @returns {LocalPiiStatus} */
function readLocalPiiStatus(response) {
  if (
    !isRecord(response) ||
    typeof response.provider_send_enabled !== "boolean" ||
    (response.detector !== "local_model" && response.detector !== "regex_only")
  ) {
    throw new Error("Local PII status unavailable");
  }
  return /** @type {LocalPiiStatus} */ (response);
}

/** @param {unknown} response */
function readLocalViewerContent(response) {
  if (!isRecord(response) || !isTokenizedText(response.tokenized) || !isOpaqueTokenReferences(response.references)) {
    throw invalidTokenizedResponse();
  }
  assertLocalViewerPayload(response.tokenized, response.references);
  return { tokenized: response.tokenized, references: response.references };
}

/** @param {unknown} response @returns {LocalSearchResult[]} */
function readLocalSearchResults(response) {
  if (!isRecord(response) || !Array.isArray(response.results) || response.results.length > 8) {
    throw new Error("Invalid local search response");
  }
  return response.results.map((result) => {
    if (!isRecord(result) || typeof result.title_token_only !== "string" ||
        result.title_token_only.trim().length === 0 || typeof result.content_token_only !== "string" ||
        result.content_token_only.trim().length === 0 || typeof result.ordinal !== "number" ||
        !Number.isInteger(result.ordinal) || result.ordinal < 0 || typeof result.score !== "number" ||
        !Number.isFinite(result.score)) {
      throw new Error("Invalid local search response");
    }
    return /** @type {LocalSearchResult} */ (result);
  });
}

/**
 * TeamOnPremise IPC is a strict allowlist: extra keys are rejected rather than
 * ignored, so a caller cannot smuggle `backend`, `path`, `token`, `password`,
 * `server`, `token_command`, or raw message text into a Tauri command.
 * @param {unknown} value @param {readonly string[]} allowed @param {string} label
 */
function assertTeamRequest(value, allowed, label) {
  if (!isRecord(value)) throw new Error(`Invalid ${label}`);
  for (const key of Object.keys(value)) {
    if (!allowed.includes(key)) throw new Error(`Invalid ${label}`);
  }
}

/** @param {unknown} value @param {string} label */
function assertTeamOpaqueRef(value, label) {
  if (
    typeof value !== "string" || value.length === 0 || value.length > 256 ||
    !/^[A-Za-z0-9:_-]+$/.test(value) || value.startsWith(".")
  ) {
    throw new Error(`Invalid ${label}`);
  }
}

/**
 * Pseudonym tokens are opaque but carry brackets in their canonical form.
 * Plaintext addresses (@, whitespace) and paths are rejected here.
 * @param {unknown} value @param {string} label
 */
function assertTeamTokenRef(value, label) {
  if (
    typeof value !== "string" || value.length === 0 || value.length > 256 ||
    !/^[A-Za-z0-9:_[\]-]+$/.test(value)
  ) {
    throw new Error(`Invalid ${label}`);
  }
}

/** @param {unknown} value @param {string} label */
function assertTeamContentFree(value, label) {
  if (Array.isArray(value)) {
    for (const item of value) assertTeamContentFree(item, label);
    return;
  }
  if (!isRecord(value)) return;
  for (const [key, child] of Object.entries(value)) {
    if (FORBIDDEN_TEAM_DTO_KEYS.includes(key)) throw new Error(`Invalid ${label}`);
    assertTeamContentFree(child, label);
  }
}

/** @param {unknown} response @returns {TeamEmailSourcesResponse} */
function readTeamSources(response) {
  assertTeamContentFree(response, "team email sources response");
  if (!isRecord(response) || !Array.isArray(response.sources) ||
      !response.sources.every((source) => isRecord(source) &&
        typeof source.source_ref === "string" &&
        Array.isArray(source.mailbox_refs) &&
        source.mailbox_refs.every((ref) => typeof ref === "string" && ref.length > 0))) {
    throw new Error("Invalid team email sources response");
  }
  return /** @type {TeamEmailSourcesResponse} */ (response);
}

/** @param {unknown} response @returns {TeamAcceptedJob} */
function readTeamAcceptedJob(response) {
  assertTeamContentFree(response, "team email job response");
  if (!isRecord(response) || typeof response.job_id !== "string" || response.job_id.length === 0) {
    throw new Error("Invalid team email job response");
  }
  return /** @type {TeamAcceptedJob} */ (response);
}

/** @param {unknown} response @returns {TeamJobStatus} */
function readTeamJobStatus(response) {
  assertTeamContentFree(response, "team email job status");
  if (!isRecord(response) || typeof response.job_id !== "string" ||
      typeof response.status !== "string" || typeof response.failed !== "boolean" ||
      (response.progress !== null && !isRecord(response.progress)) ||
      (response.result !== null && !isRecord(response.result))) {
    throw new Error("Invalid team email job status");
  }
  return /** @type {TeamJobStatus} */ (response);
}

/** @param {unknown} response @returns {TeamEmailSearchResponse} */
function readTeamSearchResponse(response) {
  assertTeamContentFree(response, "team email search response");
  if (!isRecord(response) || !Array.isArray(response.hits) ||
      !response.hits.every((hit) => isRecord(hit) &&
        typeof hit.document_ref === "string" &&
        typeof hit.external_ref === "string" &&
        typeof hit.source_kind === "string" &&
        typeof hit.ordinal === "number" && Number.isInteger(hit.ordinal) &&
        typeof hit.score === "number" && Number.isFinite(hit.score) &&
        typeof hit.projection === "string" &&
        typeof hit.viewer_policy === "string")) {
    throw new Error("Invalid team email search response");
  }
  return /** @type {TeamEmailSearchResponse} */ (response);
}

/** @param {unknown} response @returns {TeamViewerCapability} */
function readTeamCapability(response) {
  assertTeamContentFree(response, "team viewer capability");
  if (!isRecord(response) || typeof response.capability_ref !== "string" ||
      typeof response.expires_at !== "string" ||
      typeof response.document_ref !== "string" ||
      typeof response.email_ref !== "string" ||
      typeof response.delivery !== "string") {
    throw new Error("Invalid team viewer capability");
  }
  return /** @type {TeamViewerCapability} */ (response);
}

/** @param {unknown} response @returns {TeamViewerRevealResult} */
function readTeamReveal(response) {
  assertTeamContentFree(response, "team viewer reveal result");
  if (!isRecord(response) || typeof response.capability_ref !== "string" ||
      typeof response.revealed !== "boolean" ||
      typeof response.document_ref !== "string" || typeof response.email_ref !== "string") {
    throw new Error("Invalid team viewer reveal result");
  }
  return /** @type {TeamViewerRevealResult} */ (response);
}

/** @param {unknown} response @returns {TeamViewerProjection} */
function readTeamOpen(response) {
  assertTeamContentFree(response, "team viewer open projection");
  if (!isRecord(response) || typeof response.capability_ref !== "string" ||
      typeof response.document_ref !== "string" || typeof response.email_ref !== "string" ||
      typeof response.body_token_only !== "string" ||
      typeof response.projection_kind !== "string" ||
      !Array.isArray(response.headers) ||
      !response.headers.every((header) => isRecord(header) &&
        typeof header.name === "string" && typeof header.token_only === "string") ||
      !Array.isArray(response.attachment_refs) ||
      !response.attachment_refs.every((ref) => typeof ref === "string")) {
    throw new Error("Invalid team viewer open projection");
  }
  return /** @type {TeamViewerProjection} */ (response);
}

/** @param {unknown} response @returns {TeamAuditResponse} */
function readTeamAudit(response) {
  assertTeamContentFree(response, "team viewer audit response");
  if (!isRecord(response) || !Array.isArray(response.events) ||
      !response.events.every((event) => isRecord(event) &&
        typeof event.sequence === "number" && typeof event.action === "string" &&
        typeof event.outcome === "string" && typeof event.capability_ref === "string")) {
    throw new Error("Invalid team viewer audit response");
  }
  return /** @type {TeamAuditResponse} */ (response);
}

/** Provider egress follows the backend's explicit capability flag. @param {LocalPiiStatus | null | undefined} status */
export function canSendToProvider(status) {
  return status?.provider_send_enabled === true;
}

/**
 * @param {unknown} response
 * @param {string} conversationId
 * @param {string} contextId
 * @returns {TokenizedChunk[]}
 */
export function readTokenizedChunks(response, conversationId, contextId) {
  assertTokenizedResponse(response, conversationId, contextId);
  return response.chunks.map((chunk) => ({
    sequence: chunk.sequence,
    text: chunk.content.text,
    references: chunk.content.references.map(({ token, class: tokenClass }) => ({
      token,
      class: tokenClass,
    })),
  }));
}

/** @param {unknown} tokenizedText */
export function tokenizedTextToDocument(tokenizedText) {
  if (!isTokenizedText(tokenizedText)) throw invalidTokenizedResponse();

  const references = [...tokenizedText.references].sort(
    (left, right) => right.token.length - left.token.length,
  );
  const content = tokenizedText.text.split("\n").map((line) => {
    const nodes = [];
    let cursor = 0;
    while (cursor < line.length) {
      let nextIndex = -1;
      let match;
      for (const reference of references) {
        const index = line.indexOf(reference.token, cursor);
        if (index >= 0 && (nextIndex < 0 || index < nextIndex)) {
          nextIndex = index;
          match = reference;
        }
      }
      if (nextIndex < 0) {
        nodes.push({ type: "text", text: line.slice(cursor) });
        break;
      }
      if (!match) throw invalidTokenizedResponse();
      if (nextIndex > cursor) nodes.push({ type: "text", text: line.slice(cursor, nextIndex) });
      nodes.push({
        type: "text",
        text: match.token,
        marks: [{
          type: "haciendaSensitive",
          attrs: {
            tokenId: match.token,
            tokenClass: match.class,
            mode: "pseudonymize",
            source: "regex",
            disclosure: "viewer-only",
          },
        }],
      });
      cursor = nextIndex + match.token.length;
    }
    return { type: "paragraph", ...(nodes.length ? { content: nodes } : {}) };
  });

  return { type: "doc", content };
}

/**
 * @param {RendererInvoke | undefined} invoke
 * @returns {{
 *   stream: (provider: "claude" | "codex", request: ProviderRequest) => Promise<ProviderResponse>,
 *   openOriginal: (references: OpenOriginalReferences) => Promise<void>,
 *   openLocalViewer: (tokenized: TokenizedText, references: OpaqueTokenReference[]) => Promise<void>,
 *   localViewerContent: () => Promise<{ tokenized: TokenizedText, references: OpaqueTokenReference[] }>,
 *   revealLocalToken: (token: string) => Promise<string>,
 *   pseudonymizeSelection: (request: { category: string, value: string }) => Promise<TokenizedText>,
 *   localPiiStatus: () => Promise<LocalPiiStatus>,
 *   pickDocument: () => Promise<DocumentProjection>,
 *   installLocalRagModel: () => Promise<{ ready: true, model: "xberg:preset:multilingual" }>,
 *   searchLocalDocuments: (query: string) => Promise<LocalSearchResult[]>,
 *   loadDocument: (request: { documentRef: string, versionRef: string }) => Promise<DocumentProjection>,
 *   saveDocument: (request: { documentRef: string, versionRef: string, editor: Record<string, unknown> }) => Promise<{ document_id: string, version_id: string, parent_version_id: string, idempotent: boolean }>,
 *   teamEmailSources: () => Promise<TeamEmailSourcesResponse>,
   *   teamEmailIngest: (request: { sourceRef: string, mailboxRef: string, operation: string, privacyMode?: string, dossierId?: string }) => Promise<TeamAcceptedJob>,
   *   teamEmailJob: (request: { jobId: string }) => Promise<TeamJobStatus>,
   *   teamEmailJobResult: (request: { jobId: string }) => Promise<TeamJobStatus>,
   *   teamEmailSearch: (request: { sourceRef: string, mailboxRef: string, queryText: string, topK?: number, dossierId?: string }) => Promise<TeamEmailSearchResponse>,
   *   teamViewerCapability: (request: { documentRef: string, emailRef: string, tokenRef: string, attachmentRef?: string, purpose?: string }) => Promise<TeamViewerCapability>,
   *   teamViewerReveal: (request: { capabilityRef: string }) => Promise<TeamViewerRevealResult>,
   *   teamViewerOpen: (request: { capabilityRef: string }) => Promise<TeamViewerProjection>,
   *   teamViewerAudit: () => Promise<TeamAuditResponse>,
   * }}
 */
export function createRendererBridge(invoke) {
  if (typeof invoke !== "function") {
    return {
      stream: async () => {
        throw unavailable();
      },
      openOriginal: async () => {
        throw new Error("Original viewer is unavailable");
      },
      openLocalViewer: async () => {
        throw new Error("Local viewer is unavailable");
      },
      localViewerContent: async () => {
        throw new Error("Local viewer is unavailable");
      },
      revealLocalToken: async () => {
        throw new Error("Local viewer is unavailable");
      },
      pseudonymizeSelection: async () => {
        throw new Error("Local pseudonymization is unavailable");
      },
      localPiiStatus: async () => {
        throw new Error("Local PII status unavailable");
      },
      pickDocument: async () => {
        throw new Error("Local document picker is unavailable");
      },
      installLocalRagModel: async () => {
        throw new Error("Local retrieval model is unavailable");
      },
      searchLocalDocuments: async () => {
        throw new Error("Local document search is unavailable");
      },
      loadDocument: async () => {
        throw new Error("Local document loading is unavailable");
      },
      saveDocument: async () => {
        throw new Error("Local document saving is unavailable");
      },
      teamEmailSources: async () => {
        throw new Error("Team email service is unavailable");
      },
      teamEmailIngest: async () => {
        throw new Error("Team email service is unavailable");
      },
      teamEmailJob: async () => {
        throw new Error("Team email service is unavailable");
      },
      teamEmailJobResult: async () => {
        throw new Error("Team email service is unavailable");
      },
      teamEmailSearch: async () => {
        throw new Error("Team email service is unavailable");
      },
      teamViewerCapability: async () => {
        throw new Error("Team email service is unavailable");
      },
      teamViewerReveal: async () => {
        throw new Error("Team email service is unavailable");
      },
      teamViewerOpen: async () => {
        throw new Error("Team email service is unavailable");
      },
      teamViewerAudit: async () => {
        throw new Error("Team email service is unavailable");
      },
    };
  }

  return {
    async stream(provider, request) {
      const command = PROVIDER_COMMANDS[provider];
      if (!command) throw new Error("Unsupported provider");
      const response = await invoke(command, { request });
      assertTokenizedResponse(response, request.conversation_id, request.context_id);
      return response;
    },
    async openOriginal(references) {
      await invoke("open-original", {
        request: {
          document_ref: references.documentRef,
          version_ref: references.versionRef,
          original_ref: references.originalRef,
          viewer_ref: references.viewerRef,
          capability_ref: references.capabilityRef,
        },
      });
    },
    async openLocalViewer(tokenized, references) {
      assertLocalViewerPayload(tokenized, references);
      const response = await invoke("open-local-viewer", { request: { tokenized, references } });
      if (!isRecord(response) || response.opened !== true) {
        throw new Error("Local viewer is unavailable");
      }
    },
    async localViewerContent() {
      return readLocalViewerContent(await invoke("local-viewer-content"));
    },
    async revealLocalToken(token) {
      if (typeof token !== "string" || token.trim().length === 0) {
        throw new Error("Invalid local viewer token");
      }
      const value = await invoke("reveal-local-token", { token });
      if (typeof value !== "string" || value.length === 0) {
        throw new Error("Local viewer session is unavailable");
      }
      return value;
    },
    async pseudonymizeSelection(request) {
      const response = await invoke("pseudonymize-selection", { request });
      if (
        !isTokenizedText(response) ||
        response.references.length !== 1 ||
        response.text !== response.references[0].token
      ) {
        throw new Error("Local pseudonymization failed closed");
      }
      return response;
    },
    async localPiiStatus() {
      return readLocalPiiStatus(await invoke("local-pii-status"));
    },
    async pickDocument() {
      return readDocumentProjection(await invoke("pick-document"));
    },
    async installLocalRagModel() {
      const response = await invoke("install-local-rag-model");
      if (!isRecord(response) || response.ready !== true || response.model !== "xberg:preset:multilingual") {
        throw new Error("Local multilingual retrieval model is unavailable");
      }
      return /** @type {{ ready: true, model: "xberg:preset:multilingual" }} */ (response);
    },
    async searchLocalDocuments(query) {
      if (typeof query !== "string" || query.trim().length === 0 || [...query].length > 16_000) {
        throw new Error("Search query is empty or too long");
      }
      return readLocalSearchResults(await invoke("search-local-documents", { request: { query } }));
    },
    async loadDocument({ documentRef, versionRef }) {
      if (!isOpaqueReference(documentRef) || !isOpaqueReference(versionRef)) {
        throw new Error("Invalid document reference");
      }
      return readDocumentProjection(await invoke("load-document", {
        request: { document_ref: documentRef, version_ref: versionRef },
      }));
    },
    async saveDocument({ documentRef, versionRef, editor }) {
      if (!isOpaqueReference(documentRef) || !isOpaqueReference(versionRef) ||
          !isRecord(editor) || editor.type !== "doc") {
        throw new Error("Invalid document save request");
      }
      const response = await invoke("save-document", {
        request: { document_ref: documentRef, version_ref: versionRef, editor },
      });
      if (!isRecord(response) || !isOpaqueReference(response.document_id) ||
          !isOpaqueReference(response.version_id) || !isOpaqueReference(response.parent_version_id) ||
          typeof response.idempotent !== "boolean") {
        throw new Error("Invalid document save response");
      }
      return /** @type {{ document_id: string, version_id: string, parent_version_id: string, idempotent: boolean }} */ (/** @type {unknown} */ (response));
    },
    async teamEmailSources() {
      return readTeamSources(await invoke("team-email-sources"));
    },
    async teamEmailIngest({ sourceRef, mailboxRef, operation, privacyMode, dossierId }) {
      assertTeamOpaqueRef(sourceRef, "team email source reference");
      assertTeamOpaqueRef(mailboxRef, "team email mailbox reference");
      if (!TEAM_INGEST_OPERATIONS.includes(operation)) throw new Error("Invalid team email operation");
      // Build the payload from an allowlist, never by forwarding the caller's object.
      /** @type {TeamIngestRequest} */
      const request = {
        source_ref: sourceRef,
        mailbox_ref: mailboxRef,
        operation: { operation },
      };
      if (privacyMode !== undefined) {
        if (!TEAM_PRIVACY_MODES.includes(privacyMode)) throw new Error("Invalid team privacy mode");
        request.privacy_mode = privacyMode;
      }
      if (dossierId !== undefined) {
        assertTeamOpaqueRef(dossierId, "team email dossier reference");
        request.dossier_id = dossierId;
      }
      return readTeamAcceptedJob(await invoke("team-email-ingest", { request }));
    },
    async teamEmailJob({ jobId }) {
      assertTeamOpaqueRef(jobId, "team email job reference");
      return readTeamJobStatus(await invoke("team-email-job", { request: { job_id: jobId } }));
    },
    async teamEmailJobResult({ jobId }) {
      assertTeamOpaqueRef(jobId, "team email job reference");
      return readTeamJobStatus(await invoke("team-email-job-result", { request: { job_id: jobId } }));
    },
    async teamEmailSearch({ sourceRef, mailboxRef, queryText, topK, dossierId }) {
      assertTeamOpaqueRef(sourceRef, "team email source reference");
      assertTeamOpaqueRef(mailboxRef, "team email mailbox reference");
      if (typeof queryText !== "string" || queryText.trim().length === 0 ||
          [...queryText].length > 16_000) {
        throw new Error("Invalid team email search query");
      }
      /** @type {TeamSearchRequest} */
      const request = {
        source_ref: sourceRef,
        mailbox_ref: mailboxRef,
        query_text: queryText,
      };
      if (topK !== undefined) {
        if (!Number.isInteger(topK) || topK < 1 || topK > 50) throw new Error("Invalid team email search limit");
        request.top_k = topK;
      }
      if (dossierId !== undefined) {
        assertTeamOpaqueRef(dossierId, "team email dossier reference");
        request.dossier_id = dossierId;
      }
      return readTeamSearchResponse(await invoke("team-email-search", { request }));
    },
    async teamViewerCapability({ documentRef, emailRef, tokenRef, attachmentRef, purpose }) {
      assertTeamOpaqueRef(documentRef, "team document reference");
      assertTeamOpaqueRef(emailRef, "team email reference");
      assertTeamTokenRef(tokenRef, "team token reference");
      /** @type {TeamViewerCapabilityRequest} */
      const request = {
        document_ref: documentRef,
        email_ref: emailRef,
        token_ref: tokenRef,
      };
      if (attachmentRef !== undefined) {
        assertTeamOpaqueRef(attachmentRef, "team attachment reference");
        request.attachment_ref = attachmentRef;
      }
      if (purpose !== undefined) {
        assertTeamOpaqueRef(purpose, "team viewer purpose");
        request.purpose = purpose;
      }
      return readTeamCapability(await invoke("team-viewer-capability", { request }));
    },
    async teamViewerReveal({ capabilityRef }) {
      assertTeamOpaqueRef(capabilityRef, "team viewer capability");
      return readTeamReveal(await invoke("team-viewer-reveal", {
        request: { capability_ref: capabilityRef },
      }));
    },
    async teamViewerOpen({ capabilityRef }) {
      assertTeamOpaqueRef(capabilityRef, "team viewer capability");
      return readTeamOpen(await invoke("team-viewer-open", {
        request: { capability_ref: capabilityRef },
      }));
    },
    async teamViewerAudit() {
      return readTeamAudit(await invoke("team-viewer-audit"));
    },
  };
}
