# @hacienda-engine/langchain-hacienda

LangChain.js document loader for [hacienda-engine](https://github.com/jamon8888/hacienda-engine)
— redacted extraction with PII entities and audit-chain provenance as first-class
`Document` metadata.

Modeled on xberg's own `@xberg-io/langchain-xberg` loader, but calling
`POST /v1/documents` instead of raw xberg extraction, so every `Document` this yields
has already had PII removed per your server's configured redaction policy.

## Install

```bash
npm install @hacienda-engine/langchain-hacienda
```

## Usage

```typescript
import { HaciendaLoader } from "@hacienda-engine/langchain-hacienda";

const loader = new HaciendaLoader({ filePath: "contract.pdf", apiKey: "hac_..." });
const docs = await loader.load();

docs[0].pageContent; // already-redacted text
docs[0].metadata.pii_entities_found; // e.g. 3
docs[0].metadata.pii_categories; // e.g. ["EMAIL", "IBAN"]
docs[0].metadata.audit_chain_tip; // blake3 chain tip for this batch
```

Multiple files, or a whole directory, are sent as **one batched request**:

```typescript
const loader = new HaciendaLoader({ filePath: "./contracts", glob: "**/*.pdf", apiKey: "hac_..." });
```

Bytes in memory:

```typescript
const loader = new HaciendaLoader({ data: rawBytes, mimeType: "application/pdf", apiKey: "hac_..." });
```

Reuse an existing `HaciendaClient` (connection pooling across loaders, or a
non-default `baseUrl`):

```typescript
import { HaciendaClient } from "@hacienda-engine/sdk";

const client = new HaciendaClient({ baseUrl: "https://hacienda.example.com", apiKey: "hac_..." });
const loader = new HaciendaLoader({ filePath: "contract.pdf", client });
```

## RAG collections

`pushDocuments` and `HaciendaRetriever` ingest into / retrieve from a hacienda RAG
collection (`/v1/rag/collections/*`) — a completely separate part of the API from the
loader above. The collection must already exist:

```typescript
import { HaciendaLoader, HaciendaRetriever, pushDocuments } from "@hacienda-engine/langchain-hacienda";

const loader = new HaciendaLoader({ filePath: "./contracts", apiKey: "hac_..." });
const docs = await loader.load(); // already redacted

// No embeddings supplied: naive server-side full-text chunking. Only works against a
// collection provisioned with embeddingDim: 0 — see "Known gaps" below.
await pushDocuments("contracts", docs, { apiKey: "hac_..." });

const retriever = new HaciendaRetriever({ collection: "contracts", apiKey: "hac_..." });
const results = await retriever.invoke("indemnification clause");
```

With your own embeddings — required for a real, `embeddingDim > 0` vector-searchable
collection:

```typescript
const vectors = await myEmbeddingsModel.embedDocuments(docs.map((d) => d.pageContent));
await pushDocuments("contracts", docs, { embeddings: vectors, apiKey: "hac_..." });

const retriever = new HaciendaRetriever({
  collection: "contracts",
  mode: "vector",
  apiKey: "hac_...",
  retrieveExtra: { query_vector: await myEmbeddingsModel.embedQuery("indemnification") },
});
```

Neither function ever computes an embedding itself — see
[`../../LIMITATIONS.md`](../../LIMITATIONS.md) for exactly what that implies
(`embeddingDim: 0` requirement, `mode: "fulltext"`'s backend requirement, `RetrieveMode`'s
exact wire values).

### Config

| Option | Env var fallback | Default |
|---|---|---|
| `apiKey` | `HACIENDA_API_KEY` | — (required) |
| `baseUrl` | `HACIENDA_BASE_URL` | `http://127.0.0.1:8080` |

### What lands in `Document.metadata`

Same fields as `langchain-hacienda` (the Python package) — see
`../../python/langchain/README.md`'s table.

### Known gaps

See [`../../LIMITATIONS.md`](../../LIMITATIONS.md) — no per-request redaction mode
(mask vs. pseudonymize), and no tables/pages/chunking/rich metadata like xberg's own
`XbergLoader` exposes. Both are server-side (`hacienda-api`) limitations, not fixable
in this package.

## Development

```bash
cd integrations/node/langchain
npm install
npm run test:unit
npm run build
```
