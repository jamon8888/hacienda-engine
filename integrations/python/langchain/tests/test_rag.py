from __future__ import annotations

from types import SimpleNamespace
from typing import Any

import pytest
from hacienda_sdk import HaciendaApiError
from langchain_core.documents import Document
from langchain_hacienda import HaciendaLoaderError, HaciendaRetriever, push_documents

from .conftest import FakeHaciendaClient


class FakeRagNamespace:
    def __init__(
        self,
        upsert_response: Any = None,
        retrieve_response: Any = None,
        error: HaciendaApiError | None = None,
    ) -> None:
        self.upsert_response = upsert_response
        self.retrieve_response = retrieve_response
        self.error = error
        self.upsert_calls: list[Any] = []
        self.retrieve_calls: list[Any] = []

    def upsert_document(self, collection: str, body: Any) -> Any:
        self.upsert_calls.append((collection, body))
        if self.error is not None:
            raise self.error
        return self.upsert_response

    def retrieve(self, collection: str, body: Any) -> Any:
        self.retrieve_calls.append((collection, body))
        if self.error is not None:
            raise self.error
        return self.retrieve_response


class FakeRagClient(FakeHaciendaClient):
    def __init__(self, *, upsert_response: Any = None, retrieve_response: Any = None, error=None) -> None:
        super().__init__()
        self.rag = FakeRagNamespace(
            upsert_response=upsert_response, retrieve_response=retrieve_response, error=error
        )


def make_upsert_response(document_id: str) -> SimpleNamespace:
    return SimpleNamespace(document_id=document_id)


class TestPushDocuments:
    def test_sends_full_text_and_metadata_without_chunks(self):
        client = FakeRagClient(upsert_response=make_upsert_response("doc-1"))
        docs = [Document(page_content="hello redacted", metadata={"source": "a.txt", "pii_entities_found": 0})]

        ids = push_documents("my-collection", docs, client=client)

        assert ids == ["doc-1"]
        collection, body = client.rag.upsert_calls[0]
        assert collection == "my-collection"
        assert body.document["full_text"] == "hello redacted"
        assert body.document["source_uri"] == "a.txt"
        assert body.document["metadata"]["pii_entities_found"] == 0
        # No chunks sent — the server naive-chunks full_text itself.
        from hacienda_sdk._generated.types import Unset

        assert isinstance(body.chunks, Unset)

    def test_sends_supplied_embeddings_as_a_single_chunk(self):
        client = FakeRagClient(upsert_response=make_upsert_response("doc-2"))
        docs = [Document(page_content="vectorizable text")]

        push_documents("coll", docs, embeddings=[[0.1, 0.2, 0.3]], client=client)

        _, body = client.rag.upsert_calls[0]
        assert body.chunks == [
            {"ordinal": 0, "content": "vectorizable text", "embedding": [0.1, 0.2, 0.3]}
        ]

    def test_rejects_mismatched_embeddings_length(self):
        client = FakeRagClient()
        docs = [Document(page_content="a"), Document(page_content="b")]
        with pytest.raises(HaciendaLoaderError, match="same length"):
            push_documents("coll", docs, embeddings=[[0.1]], client=client)

    def test_batches_multiple_documents_as_separate_calls(self):
        client = FakeRagClient(upsert_response=make_upsert_response("doc-x"))
        docs = [Document(page_content="a"), Document(page_content="b")]

        ids = push_documents("coll", docs, client=client)

        assert ids == ["doc-x", "doc-x"]
        assert len(client.rag.upsert_calls) == 2

    def test_wraps_api_errors(self):
        client = FakeRagClient(error=HaciendaApiError(403, "forbidden", "no rag:write"))
        with pytest.raises(HaciendaLoaderError, match="hacienda-engine request failed"):
            push_documents("coll", [Document(page_content="x")], client=client)

    def test_closes_owned_client(self, monkeypatch):
        fake_client = FakeRagClient(upsert_response=make_upsert_response("doc-1"))
        monkeypatch.setenv("HACIENDA_API_KEY", "test-key")
        monkeypatch.setattr("langchain_hacienda.rag.HaciendaClient", lambda *a, **kw: fake_client)

        push_documents("coll", [Document(page_content="x")])

        assert fake_client.closed is True

    def test_does_not_close_caller_supplied_client(self):
        client = FakeRagClient(upsert_response=make_upsert_response("doc-1"))
        push_documents("coll", [Document(page_content="x")], client=client)
        assert client.closed is False


class TestHaciendaRetriever:
    def test_maps_chunks_to_documents(self):
        retrieve_response = {
            "mode": "fulltext",
            "chunks": [
                {
                    "id": "doc-1:0",
                    "document_id": "doc-1",
                    "ordinal": 0,
                    "content": "Contract text",
                    "score": 0.87,
                    "primary_score": {"kind": "fulltext", "score": 0.87},
                    "chunk_metadata": {"heading": "Intro"},
                }
            ],
            "primary_latency_ms": 3,
        }
        client = FakeRagClient(retrieve_response=retrieve_response)
        retriever = HaciendaRetriever(collection="my-collection", client=client)

        docs = retriever.invoke("acme contract")

        assert len(docs) == 1
        doc = docs[0]
        assert doc.page_content == "Contract text"
        assert doc.metadata["score"] == 0.87
        assert doc.metadata["document_id"] == "doc-1"
        assert doc.metadata["chunk_id"] == "doc-1:0"
        assert doc.metadata["heading"] == "Intro"

        collection, body = client.rag.retrieve_calls[0]
        assert collection == "my-collection"
        assert body["mode"] == "fulltext"
        assert body["query_text"] == "acme contract"
        assert body["top_k"] == 10
        assert body["include_content"] is True

    def test_empty_result_returns_no_documents(self):
        client = FakeRagClient(retrieve_response={"mode": "vector", "chunks": [], "primary_latency_ms": 1})
        retriever = HaciendaRetriever(collection="coll", client=client)

        docs = retriever.invoke("nothing matches")

        assert docs == []

    def test_custom_mode_and_top_k(self):
        client = FakeRagClient(retrieve_response={"mode": "vector", "chunks": [], "primary_latency_ms": 1})
        retriever = HaciendaRetriever(collection="coll", mode="vector", top_k=3, client=client)

        retriever.invoke("q")

        _, body = client.rag.retrieve_calls[0]
        assert body["mode"] == "vector"
        assert body["top_k"] == 3

    def test_retrieve_kwargs_pass_through(self):
        client = FakeRagClient(retrieve_response={"mode": "vector", "chunks": [], "primary_latency_ms": 1})
        retriever = HaciendaRetriever(
            collection="coll", mode="vector", client=client, retrieve_kwargs={"query_vector": [0.1, 0.2]}
        )

        retriever.invoke("q")

        _, body = client.rag.retrieve_calls[0]
        assert body["query_vector"] == [0.1, 0.2]

    def test_wraps_api_errors(self):
        client = FakeRagClient(error=HaciendaApiError(400, "invalid_request", "bad query"))
        retriever = HaciendaRetriever(collection="coll", client=client)
        with pytest.raises(HaciendaLoaderError, match="hacienda-engine request failed"):
            retriever.invoke("q")
