# Known limitations

This is the one place gaps in `integrations/` are tracked — package READMEs link
here rather than restating them, so there's a single source of truth.

Two categories: things that are **server-side limitations** (no client-integration
package can close them without a `hacienda-api` change), and things that are
**deliberate scope decisions** in how an integration was built. Both are listed
plainly below, with the DTO/code evidence for the server-side ones so they're
checkable, not just asserted.

## `langchain-hacienda` (Python + Node)

Wraps `POST /v1/documents` — see `python/langchain/README.md` /
`node/langchain/README.md` for what it does cover.

### Server-side: no per-request redaction mode

There is no way to choose mask vs. pseudonymize (or anything else) per call. Every
request DTO involved — `RedactTextRequest`, `ScanTextRequest`, `DocumentInput` — is
`#[serde(deny_unknown_fields)]` and has no `mode`/`redaction_mode`/`preset_id` field
(`hacienda-api/src/dto.rs`). A request that tries to pass one is rejected with 400,
not silently ignored. Redaction mode is fixed at server-config time
(`RedactionConfig::mode` in `hacienda-core/src/redaction/types.rs`) — `GET
/v1/pii/config` can *read* the active mode, nothing can set it per-request. Presets
(`/v1/presets`) exist as a CRUD resource but are not wired into the redaction pipeline
anywhere in the current codebase — creating one has no effect on `/v1/documents` or
`/v1/pii/redact`.

**Implication**: if your hacienda deployment is configured for `Mask` (the default),
every document this loader returns is masked — you cannot ask for pseudonymized
output from the client.

### Server-side: no rich extraction metadata

`POST /v1/documents`'s response (`DocumentResult` in `dto.rs`) is exactly
`{ content, entities, document_id?, version_sequence? }`. It does not return: tables,
per-page splitting, chunking, document metadata (title/authors/keywords/language/
dates), quality score, detected languages, or extraction warnings — all things
xberg's own `XbergLoader` (the loader this one is modeled on) exposes directly,
because it talks to xberg's raw `ExtractedDocument` instead of hacienda's redacting
wrapper around it. Closing this gap means extending `hacienda-api`'s response DTO,
not something fixable in an integration package.

### Not a gap — already covered without new code

Revealing a pseudonymized token back to plaintext (`POST /v1/pii/reveal`) is already
usable directly via `HaciendaClient.pii.reveal_token(...)` on both SDKs, gated only by
the `pii:reveal` capability on the caller's API key — confirmed **not** coupled to the
review queue (`/v1/review`) at all; those are two independent authorization surfaces
in `hacienda-core/src/facade.rs`. No wrapper needed in this package.

## RAG integration (`push_documents`/`pushDocuments`, `HaciendaRetriever`)

Verified end-to-end against a live `hacienda serve` instance (both languages) —
`push_documents` → `HaciendaRetriever.invoke()` round-trips real content in both the
no-embeddings and supplied-embeddings paths. Operational details below were all
confirmed live, not assumed from reading the Rust source alone — two of them
(`RetrieveMode`'s exact wire values, and the `embedding_dim: 0` requirement) turned
out different from what the source comments alone suggested.

- **`RetrieveMode`'s wire values have no underscore.** `#[serde(rename_all =
  "lowercase")]` on `hacienda_rag::RetrieveMode` — the values are `"vector"`
  (default), `"fulltext"`, `"hybrid"`, `"sparse"`, `"lateinteraction"`. Not
  `"full_text"`/`"late_interaction"` — sending those is rejected as invalid JSON for
  the endpoint.
- **No client-side embedding generation, by design.** Neither `push_documents` nor
  `HaciendaRetriever` computes an embedding. `push_documents` accepts an optional
  `embeddings` parameter (one vector per document) if you already have one (e.g. from
  a LangChain `Embeddings` model) — it still never computes one itself, only forwards
  what you give it as a single whole-document chunk.
- **Without `embeddings`, the target collection must have `embedding_dim: 0`.**
  Verified live: omitting `chunks` on `UpsertDocumentRequest` makes the server
  naive-chunk `full_text` itself with an empty embedding attached
  (`hacienda_rag::chunk_full_text`) — upserting that into a collection provisioned
  with `embedding_dim > 0` fails with `HTTP 400: embedding dimension mismatch:
  expected N, got 0`. There is no partial/mixed state: a collection is either
  `embedding_dim: 0` (naive ingest works, vector retrieval is meaningless) or
  `embedding_dim > 0` (naive ingest fails outright, `embeddings=` is required).
- **`mode="fulltext"` retrieval requires a full-text-capable backend.** The in-memory
  `RagStore` backend (what a bare `hacienda serve` uses without further config)
  reports `Capabilities { full_text: false, .. }` and any `"fulltext"`/`"hybrid"`
  request against it fails server-side with "retrieval mode unsupported by backend"
  (confirmed live). Only the pgvector backend (`Capabilities { full_text: true, .. }`)
  supports it. **Not independently verified live in this session** — no pgvector
  instance was available — so treat the pgvector code path as read-from-source, not
  smoke-tested, unlike the vector-mode path above.
- **No PII redaction on the RAG ingest path.** Confirmed in
  `crates/hacienda-rag/src/store.rs`: `RagStore::upsert_document` is a pure storage
  write, no text scanning. If you push raw (non-redacted) text into a collection via
  `push_documents`, it is stored raw. Feed it `HaciendaLoader.load()`'s output (already
  redacted) rather than arbitrary text if that matters to you. The one place hacienda
  *does* mandate redaction is `POST /v1/rag/collections/{name}/answer` — the streaming
  answer-synthesis endpoint redacts the prompt and every retrieved chunk server-side
  before either reaches an LLM, but nothing in this package wraps that endpoint (see
  below).
- **No answer-synthesis wrapper.** `POST /v1/rag/collections/{name}/answer` (SSE
  streaming, grounded LLM answers) isn't wrapped by this package — it doesn't map
  cleanly onto LangChain's `Runnable`/`BaseChatModel` interfaces without a
  meaningfully larger integration surface than a retriever needs. Tracked as future
  work, not started.

### Bug found and fixed along the way: `hacienda-sdk` (Python) silently discarded RAG responses

`retrieve`, `create_collection`, and `get_collection` are declared `body =
serde_json::Value` in their `#[utoipa::path]` responses (their `hacienda_rag` types
aren't `utoipa::ToSchema`-registered), which resolves to an empty `{}` OpenAPI schema.
`openapi-python-client`'s generated response parser for an empty schema unconditionally
returns `None` on 2xx — discarding the real response body entirely, even though
`response.content` had the actual JSON in it. `client.rag.retrieve(...)` therefore
always returned `None`, previously — confirmed by generating the real client from a
live server and hitting exactly this. Fixed in `sdks/python/src/hacienda_sdk/errors.py`
(`_unwrap_json`, falls back to parsing `response.content` when `.parsed` is empty on a
2xx) and wired into the three affected `client.py` methods. The TypeScript SDK does not
have this problem — `openapi-fetch` always calls `response.json()` regardless of schema
richness, just with weaker (`unknown`) compile-time types for these three routes.

## Planned packages (not yet implemented)

`python/llama-index`, `python/crewai`, `python/txtai`, `node/n8n-nodes-hacienda`,
`java/spring-ai` — see each one's own README for what it would do; none has adapter
code yet.
