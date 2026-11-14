const PROVIDER_COMMANDS = Object.freeze({
  claude: "stream-claude",
  codex: "stream-codex",
});

const unavailable = () => new Error("Desktop provider is unavailable");

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

export function normalizePiiClass(value) {
  if (typeof value !== "string") return "sensitive";
  const key = value
    .replace(/([a-z0-9])([A-Z])/g, "$1_$2")
    .toLowerCase()
    .replace(/[^a-z0-9]/g, "");
  return PII_CLASS_ALIASES[key] ?? "sensitive";
}

export function createTurnRequest({ conversationId, contextId, prompt, context = [] }) {
  return {
    conversation_id: conversationId,
    context_id: contextId,
    prompt,
    context,
  };
}

export function readTokenizedChunks(response, conversationId, contextId) {
  if (
    !response ||
    response.conversation_id !== conversationId ||
    response.context_id !== contextId ||
    !Array.isArray(response.chunks)
  ) {
    throw new Error("Invalid tokenized response");
  }

  return response.chunks.map((chunk) => {
    if (
      !chunk ||
      chunk.conversation_id !== conversationId ||
      chunk.context_id !== contextId ||
      !Number.isSafeInteger(chunk.sequence) ||
      !chunk.content ||
      typeof chunk.content.text !== "string" ||
      !Array.isArray(chunk.content.references) ||
      chunk.content.references.some(
        (reference) =>
          !reference ||
          typeof reference.token !== "string" ||
          typeof reference.class !== "string",
      )
    ) {
      throw new Error("Invalid tokenized response");
    }

    return {
      sequence: chunk.sequence,
      text: chunk.content.text,
      references: chunk.content.references.map(({ token, class: tokenClass }) => ({
        token,
        class: tokenClass,
      })),
    };
  });
}

export function tokenizedTextToDocument(tokenizedText) {
  if (
    !tokenizedText ||
    typeof tokenizedText.text !== "string" ||
    !Array.isArray(tokenizedText.references) ||
    tokenizedText.references.some(
      (reference) =>
        !reference ||
        typeof reference.token !== "string" ||
        reference.token.length === 0 ||
        typeof reference.class !== "string",
    )
  ) {
    throw new Error("Invalid tokenized response");
  }

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
            source: "provider",
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

export function createRendererBridge(invoke) {
  if (typeof invoke !== "function") {
    return {
      stream: async () => {
        throw unavailable();
      },
      openOriginal: async () => {
        throw new Error("Original viewer is unavailable");
      },
    };
  }

  return {
    async stream(provider, request) {
      const command = PROVIDER_COMMANDS[provider];
      if (!command) throw new Error("Unsupported provider");
      return invoke(command, { request });
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
  };
}
