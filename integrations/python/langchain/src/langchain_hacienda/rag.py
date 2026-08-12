"""hacienda-engine RAG collection integration for LangChain: ingest + retrieve.

Deliberately does not generate embeddings client-side — see this module's docstrings
and `../../LIMITATIONS.md` for what that means in practice (in short: ingestion always
uses the server's naive full-text chunking, and the default retrieval mode,
``"fulltext"``, only works against a full-text-capable `RagStore` backend — the
in-memory backend does not support it. A caller with their own embeddings can pass
``mode="vector"`` plus a ``query_vector`` through `HaciendaRetriever` directly).
"""

from __future__ import annotations

import os
from typing import TYPE_CHECKING, Any

from hacienda_sdk import HaciendaApiError, HaciendaClient
from hacienda_sdk._generated.models.upsert_document_request import UpsertDocumentRequest
from langchain_core.callbacks import CallbackManagerForRetrieverRun
from langchain_core.documents import Document
from langchain_core.retrievers import BaseRetriever

from langchain_hacienda.loader import HaciendaLoaderError

if TYPE_CHECKING:
    from collections.abc import Sequence


def _resolve_client(
    *,
    client: HaciendaClient | None,
    base_url: str | None,
    api_key: str | None,
) -> tuple[HaciendaClient, bool]:
    """Shared client-resolution logic with `HaciendaLoader.__init__`.

    Returns ``(client, owns_client)`` — ``owns_client`` is True only when this
    function constructed the client itself (no ``client=`` was passed), mirroring
    `HaciendaLoader`'s own ownership tracking for deterministic `close()`.
    """
    if client is not None:
        return client, False

    resolved_base_url = base_url or os.environ.get("HACIENDA_BASE_URL", "http://127.0.0.1:8080")
    resolved_api_key = api_key or os.environ.get("HACIENDA_API_KEY")
    if not resolved_api_key:
        raise HaciendaLoaderError(
            "No API key: pass api_key=..., set HACIENDA_API_KEY, or pass an "
            "existing client=HaciendaClient(...)."
        )
    return HaciendaClient(base_url=resolved_base_url, api_key=resolved_api_key), True


def push_documents(
    collection: str,
    documents: Sequence[Document],
    *,
    embeddings: Sequence[Sequence[float]] | None = None,
    client: HaciendaClient | None = None,
    base_url: str | None = None,
    api_key: str | None = None,
) -> list[str]:
    """Ingest `Document`s into a hacienda RAG collection.

    The collection must already exist (`POST /v1/rag/collections` — provisioning one,
    choosing its `embedding_dim`/`distance_metric`/`index_method`, is a deployment
    decision, not something this function makes for you).

    No PII redaction happens here — `RagStore::upsert_document` is a pure storage
    write (`crates/hacienda-rag/src/store.rs`). Feed this `HaciendaLoader.load()`'s
    output (already redacted), not arbitrary raw text, if that matters to you.

    Without ``embeddings``, each document is sent with no pre-computed ``chunks``, so
    the server naive-chunks `full_text` itself (`hacienda_rag::chunk_full_text`) with
    no embeddings attached. **This only succeeds against a collection provisioned with
    `embedding_dim: 0`** — confirmed live: upserting a chunkless document into a
    collection with `embedding_dim > 0` fails with "embedding dimension mismatch:
    expected N, got 0", since an omitted-chunks upsert always produces zero-length
    embeddings. A `embedding_dim: 0` collection is enough for `mode="fulltext"`
    retrieval (itself only supported by a full-text-capable backend — see
    `HaciendaRetriever`), not for `mode="vector"`.

    Pass ``embeddings`` (one vector per document, same order, matching the
    collection's `embedding_dim`) to populate a real vector-searchable collection
    instead — this function still never computes an embedding itself, it only accepts
    one you already have (e.g. from a LangChain `Embeddings` model).

    Args:
        collection: Name of an existing RAG collection.
        documents: Documents to ingest — `Document.page_content` becomes
            `DocumentRecord.full_text`; `Document.metadata` is passed through as
            `DocumentRecord.metadata`, and `metadata["source"]` (if present, e.g. from
            `HaciendaLoader`) becomes `DocumentRecord.source_uri`.
        embeddings: One embedding vector per document (same order), if you have your
            own. Sent as a single whole-document chunk with that embedding attached.
            Omit to use naive server-side full-text chunking instead (see above).
        client: A pre-built client. Takes precedence over `base_url`/`api_key`.
        base_url: Falls back to `HACIENDA_BASE_URL`, then `http://127.0.0.1:8080`.
        api_key: Falls back to `HACIENDA_API_KEY`.

    Returns:
        The server-assigned `document_id` for each input document, same order.

    Raises:
        HaciendaLoaderError: Wraps any `HaciendaApiError` raised by a request, or if
            `embeddings` is given with a different length than `documents`.
    """
    if embeddings is not None and len(embeddings) != len(documents):
        raise HaciendaLoaderError(
            f"'embeddings' has {len(embeddings)} vectors but 'documents' has "
            f"{len(documents)} — they must be the same length, same order."
        )

    resolved_client, owns_client = _resolve_client(client=client, base_url=base_url, api_key=api_key)
    try:
        document_ids: list[str] = []
        for index, doc in enumerate(documents):
            record: dict[str, Any] = {"full_text": doc.page_content}
            if doc.metadata:
                record["metadata"] = doc.metadata
                source = doc.metadata.get("source")
                if isinstance(source, str):
                    record["source_uri"] = source

            if embeddings is not None:
                body = UpsertDocumentRequest(
                    document=record,
                    chunks=[
                        {"ordinal": 0, "content": doc.page_content, "embedding": list(embeddings[index])}
                    ],
                )
            else:
                body = UpsertDocumentRequest(document=record)

            try:
                response = resolved_client.rag.upsert_document(collection, body)
            except HaciendaApiError as exc:
                raise HaciendaLoaderError(f"hacienda-engine request failed: {exc}") from exc
            document_ids.append(response.document_id)
        return document_ids
    finally:
        if owns_client:
            resolved_client.close()


class HaciendaRetriever(BaseRetriever):
    """Retrieve from a hacienda RAG collection (`POST /v1/rag/collections/{name}/retrieve`).

    Defaults to ``mode="fulltext"`` — the pairing that needs no client-side embedding
    generation, matching `push_documents`'s naive-chunking ingest. This **only works
    against a full-text-capable `RagStore` backend** (e.g. the pgvector backend); the
    in-memory backend used by a bare `hacienda serve` reports
    ``Capabilities { full_text: false, .. }`` and the request fails server-side with
    "retrieval mode unsupported by backend". Pass ``mode="vector"`` with your own
    ``query_vector`` (see `retrieve_kwargs`) to use the in-memory backend, or any
    backend without full-text support. See ``../../LIMITATIONS.md``.
    """

    collection: str
    top_k: int = 10
    mode: str = "fulltext"
    retrieve_kwargs: dict[str, Any] = {}
    client: Any = None
    base_url: str | None = None
    api_key: str | None = None

    def _get_relevant_documents(
        self, query: str, *, run_manager: CallbackManagerForRetrieverRun
    ) -> list[Document]:
        resolved_client, owns_client = _resolve_client(
            client=self.client, base_url=self.base_url, api_key=self.api_key
        )
        try:
            body: dict[str, Any] = {
                "mode": self.mode,
                "query_text": query,
                "top_k": self.top_k,
                "include_content": True,
                "include_document": True,
                **self.retrieve_kwargs,
            }
            try:
                # `retrieve`'s OpenAPI response schema is `serde_json::Value` (see
                # `hacienda_sdk.errors._unwrap_json`'s docstring) — this returns a
                # plain dict, not an attrs model, unlike `documents.process_documents`.
                result = resolved_client.rag.retrieve(self.collection, body)
            except HaciendaApiError as exc:
                raise HaciendaLoaderError(f"hacienda-engine request failed: {exc}") from exc

            chunks = result.get("chunks", []) if result else []
            documents: list[Document] = []
            for chunk in chunks:
                metadata: dict[str, Any] = {
                    "score": chunk.get("score"),
                    "document_id": chunk.get("document_id"),
                    "chunk_id": chunk.get("id"),
                    "ordinal": chunk.get("ordinal"),
                }
                chunk_metadata = chunk.get("chunk_metadata")
                if isinstance(chunk_metadata, dict):
                    metadata.update(chunk_metadata)
                documents.append(
                    Document(page_content=chunk.get("content") or "", metadata=metadata)
                )
            return documents
        finally:
            if owns_client:
                resolved_client.close()
