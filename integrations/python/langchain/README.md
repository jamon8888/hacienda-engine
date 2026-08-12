# langchain-hacienda

LangChain document loader for [hacienda-engine](https://github.com/jamon8888/hacienda-engine) —
redacted extraction with PII entities and audit-chain provenance as first-class
`Document` metadata.

Modeled on xberg's own `langchain-xberg` loader, but calling `POST /v1/documents`
instead of raw xberg extraction, so every `Document` this yields has already had PII
removed per your server's configured redaction policy — no separate redaction step
needed before the content reaches a vector store or an LLM context.

## Install

```bash
pip install langchain-hacienda
```

## Usage

```python
from langchain_hacienda import HaciendaLoader

# HaciendaLoader owns and closes its client deterministically as a context manager
# when you don't pass client=... yourself (see "Reuse an existing client" below).
with HaciendaLoader(file_path="contract.pdf", api_key="hac_...") as loader:
    docs = loader.load()

docs[0].page_content       # already-redacted text
docs[0].metadata["pii_entities_found"]  # e.g. 3
docs[0].metadata["pii_categories"]      # e.g. ["EMAIL", "IBAN"]
docs[0].metadata["audit_chain_tip"]     # blake3 chain tip for this batch
```

Multiple files, or a whole directory, are sent as **one batched request** — batching
happens server-side, not as N sequential client calls:

```python
loader = HaciendaLoader(file_path="./contracts/", glob="**/*.pdf", api_key="hac_...")
docs = loader.load()
```

Bytes in memory:

```python
loader = HaciendaLoader(data=raw_bytes, mime_type="application/pdf", api_key="hac_...")
```

Reuse an existing `hacienda_sdk.HaciendaClient` (connection pooling across loaders, or a
non-default `base_url`). A caller-supplied client is never closed by the loader — you
own its lifecycle:

```python
from hacienda_sdk import HaciendaClient

with HaciendaClient(base_url="https://hacienda.example.com", api_key="hac_...") as client:
    loader = HaciendaLoader(file_path="contract.pdf", client=client)
    docs = loader.load()
```

Enable server-side document versioning (single-file loads only — see
`GET /v1/documents/{id}/versions`):

```python
loader = HaciendaLoader(file_path="contract.pdf", document_id=my_uuid, api_key="hac_...")
```

## RAG collections

`push_documents` and `HaciendaRetriever` ingest into / retrieve from a hacienda RAG
collection (`/v1/rag/collections/*`) — a completely separate part of the API from the
loader above. The collection must already exist (provisioning one is a deployment
decision, not something either function makes for you):

```python
from langchain_hacienda import HaciendaLoader, HaciendaRetriever, push_documents

with HaciendaLoader(file_path="./contracts/", api_key="hac_...") as loader:
    docs = loader.load()  # already redacted

# No embeddings supplied: naive server-side full-text chunking. Only works against a
# collection provisioned with embedding_dim: 0 — see "Known gaps" below.
push_documents("contracts", docs, api_key="hac_...")

retriever = HaciendaRetriever(collection="contracts", api_key="hac_...")
results = retriever.invoke("indemnification clause")
```

With your own embeddings (e.g. from a LangChain `Embeddings` model) — required for a
real, `embedding_dim > 0` vector-searchable collection:

```python
vectors = my_embeddings_model.embed_documents([d.page_content for d in docs])
push_documents("contracts", docs, embeddings=vectors, api_key="hac_...")

retriever = HaciendaRetriever(
    collection="contracts",
    mode="vector",
    api_key="hac_...",
    retrieve_kwargs={"query_vector": my_embeddings_model.embed_query("indemnification")},
)
```

Neither function ever computes an embedding itself — see
[`../../LIMITATIONS.md`](../../LIMITATIONS.md) for exactly what that implies
(`embedding_dim: 0` requirement, `mode="fulltext"`'s backend requirement, `RetrieveMode`'s
exact wire values).

### Config

| Param | Env var fallback | Default |
|---|---|---|
| `api_key` | `HACIENDA_API_KEY` | — (required) |
| `base_url` | `HACIENDA_BASE_URL` | `http://127.0.0.1:8080` |

### What lands in `Document.metadata`

| Key | Present when | Notes |
|---|---|---|
| `source` | always | file path, or `bytes://<mime_type>` |
| `pii_entities_found` | always | count |
| `pii_categories` | always | sorted unique categories, e.g. `["EMAIL", "IBAN"]` |
| `pii_entities` | always | full list: `category`, `start`, `end`, `confidence`, `source` — never raw span text (`POST /v1/documents` never sets `include_text`) |
| `audit_chain_tip` | auditing enabled server-side | this batch's blake3 chain tip |
| `document_id`, `version_sequence` | `document_id=...` was passed | echoes the server-assigned version |
| `processing_time_ms` | always | server-side processing time for the whole batch |

### Known gaps

See [`../../LIMITATIONS.md`](../../LIMITATIONS.md) — no per-request redaction mode
(mask vs. pseudonymize), and no tables/pages/chunking/rich metadata like xberg's own
`XbergLoader` exposes. Both are server-side (`hacienda-api`) limitations, not fixable
in this package.

## Development

```bash
cd integrations/python/langchain
uv sync

# hacienda_sdk's own generated client must exist first (gitignored, regenerated
# on demand — see sdks/python/README.md):
cd ../../../sdks/python && bash scripts/generate.sh && cd -

uv run pytest
```

## Why this isn't just xberg's `XbergLoader` with a different import

xberg's loader extracts raw text. This loader's entire reason to exist is that the
content it returns has **already been redacted server-side**, with a verifiable audit
trail — see `../../README.md` for the full rationale.
