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

loader = HaciendaLoader(file_path="contract.pdf", api_key="hac_...")
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
non-default `base_url`):

```python
from hacienda_sdk import HaciendaClient

client = HaciendaClient(base_url="https://hacienda.example.com", api_key="hac_...")
loader = HaciendaLoader(file_path="contract.pdf", client=client)
```

Enable server-side document versioning (single-file loads only — see
`GET /v1/documents/{id}/versions`):

```python
loader = HaciendaLoader(file_path="contract.pdf", document_id=my_uuid, api_key="hac_...")
```

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
