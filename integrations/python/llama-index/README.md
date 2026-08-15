# llama-index-readers-hacienda

**Status: Planned, not yet implemented.**

## What this would be

A LlamaIndex `BaseReader` calling `POST /v1/documents`, same pattern as
`../langchain/README.md`'s `HaciendaLoader` — one `Document` per input (or per
chunk/page, mirroring xberg's `llama-index-readers-xberg` and
`llama-index-node-parser-xberg` options), redacted content, PII entities and
`audit_chain_tip` as metadata.

The more interesting piece, beyond a reader: a LlamaIndex `VectorStore` /
`BaseRetriever` implementation that proxies directly to
`POST /v1/rag/collections/{name}/retrieve` and `POST /v1/rag/collections/{name}/answer`
instead of embedding+storing locally — `hacienda-rag` (`crates/hacienda-rag`) already
owns chunking, embedding, and a pgvector-backed store server-side, so a LlamaIndex user
could point at a hacienda collection as their vector index with no local vector DB at
all, same audited/redacted guarantee end to end.

## Porting checklist (once prioritized)

1. `HaciendaReader(BaseReader)` — mirror `langchain-hacienda`'s `HaciendaLoader`.
2. `HaciendaVectorStore` — wraps `client.rag.upsert_document` / `client.rag.retrieve`.
3. Tests against a live `hacienda serve` instance (see
   `../langchain/README.md`'s Development section for the pattern).

## Reference

xberg's own `integrations/python/llama-index/` (readers + node_parsers) for the
extraction-only version this would parallel.
