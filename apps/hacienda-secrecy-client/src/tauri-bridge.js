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
/** @typedef {{ request: ProviderRequest | OpenOriginalRequest | OpenLocalViewerRequest | { category: string, value: string } | { document_ref: string, version_ref: string } | { document_ref: string, version_ref: string, editor: Record<string, unknown> } } | { token: string }} InvokePayload */
/** @typedef {(command: "stream-claude" | "stream-codex" | "open-original" | "open-local-viewer" | "local-viewer-content" | "reveal-local-token" | "pseudonymize-selection" | "local-pii-status" | "pick-document" | "load-document" | "save-document", payload?: InvokePayload) => Promise<unknown>} RendererInvoke */
/** @typedef {{ sequence: number, text: string, references: OpaqueTokenReference[] }} TokenizedChunk */

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
 *   loadDocument: (request: { documentRef: string, versionRef: string }) => Promise<DocumentProjection>,
 *   saveDocument: (request: { documentRef: string, versionRef: string, editor: Record<string, unknown> }) => Promise<{ document_id: string, version_id: string, parent_version_id: string, idempotent: boolean }>,
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
      loadDocument: async () => {
        throw new Error("Local document loading is unavailable");
      },
      saveDocument: async () => {
        throw new Error("Local document saving is unavailable");
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
  };
}
