from __future__ import annotations

from uuid import UUID, uuid4

import pytest
from hacienda_sdk import HaciendaApiError
from langchain_hacienda import HaciendaLoader, HaciendaLoaderError

from .conftest import make_entity, make_response, make_result


def test_requires_file_path_or_data(fake_client_factory):
    with pytest.raises(HaciendaLoaderError, match="file_path.*or.*data"):
        HaciendaLoader(client=fake_client_factory())


def test_rejects_both_file_path_and_data(tmp_path, fake_client_factory):
    path = tmp_path / "a.txt"
    path.write_text("hi")
    with pytest.raises(HaciendaLoaderError, match="both"):
        HaciendaLoader(file_path=path, data=b"hi", client=fake_client_factory())


def test_data_requires_mime_type(fake_client_factory):
    with pytest.raises(HaciendaLoaderError, match="mime_type"):
        HaciendaLoader(data=b"hi", client=fake_client_factory())


def test_missing_api_key_raises(monkeypatch):
    monkeypatch.delenv("HACIENDA_API_KEY", raising=False)
    with pytest.raises(HaciendaLoaderError, match="API key"):
        HaciendaLoader(data=b"hi", mime_type="text/plain")


def test_document_id_rejected_for_batch(tmp_path, fake_client_factory):
    (tmp_path / "a.txt").write_text("a")
    (tmp_path / "b.txt").write_text("b")
    with pytest.raises(HaciendaLoaderError, match="single-file"):
        HaciendaLoader(
            file_path=[tmp_path / "a.txt", tmp_path / "b.txt"],
            document_id=str(uuid4()),
            client=fake_client_factory(),
        )


def test_loads_single_file_with_redacted_content_and_metadata(tmp_path, fake_client_factory):
    path = tmp_path / "note.txt"
    path.write_text("Contact [EMAIL_REDACTED] for details.")

    entity = make_entity(category="EMAIL", start=8, end=25, confidence=0.99, source="regex")
    result = make_result("Contact [EMAIL_REDACTED] for details.", entities=[entity])
    response = make_response([result], audit_chain_tip="blake3:abc123")
    client = fake_client_factory(response=response)

    loader = HaciendaLoader(file_path=path, client=client)
    docs = loader.load()

    assert len(docs) == 1
    doc = docs[0]
    assert doc.page_content == "Contact [EMAIL_REDACTED] for details."
    assert doc.metadata["source"] == str(path)
    assert doc.metadata["pii_entities_found"] == 1
    assert doc.metadata["pii_categories"] == ["EMAIL"]
    assert doc.metadata["pii_entities"][0]["category"] == "EMAIL"
    assert doc.metadata["audit_chain_tip"] == "blake3:abc123"
    assert doc.metadata["processing_time_ms"] == 12

    # MIME type was inferred from the .txt extension and sent to the API.
    sent = client.documents.calls[0]
    assert sent.documents[0].mime_type == "text/plain"


def test_loads_from_bytes(fake_client_factory):
    result = make_result("hello redacted", entities=[])
    response = make_response([result])
    client = fake_client_factory(response=response)

    loader = HaciendaLoader(
        data=b"hello world", mime_type="text/plain", filename="mem.txt", client=client
    )
    docs = loader.load()

    assert len(docs) == 1
    assert docs[0].metadata["source"] == "bytes://text/plain"
    assert docs[0].metadata["pii_entities_found"] == 0
    assert docs[0].metadata["pii_categories"] == []


def test_batches_a_directory_in_one_request(tmp_path, fake_client_factory):
    (tmp_path / "a.txt").write_text("a")
    (tmp_path / "b.txt").write_text("b")

    results = [make_result("a redacted"), make_result("b redacted")]
    response = make_response(results)
    client = fake_client_factory(response=response)

    loader = HaciendaLoader(file_path=tmp_path, client=client)
    docs = loader.load()

    assert len(docs) == 2
    assert len(client.documents.calls) == 1  # one batched request, not two
    assert {doc.page_content for doc in docs} == {"a redacted", "b redacted"}


def test_carries_document_id_and_version_sequence(tmp_path, fake_client_factory):
    path = tmp_path / "v.txt"
    path.write_text("v1")
    doc_id = uuid4()

    result = make_result("v1 redacted", document_id=doc_id, version_sequence=3)
    response = make_response([result])
    client = fake_client_factory(response=response)

    loader = HaciendaLoader(file_path=path, document_id=doc_id, client=client)
    doc = loader.load()[0]

    assert doc.metadata["document_id"] == str(doc_id)
    assert doc.metadata["version_sequence"] == 3

    sent = client.documents.calls[0]
    assert sent.documents[0].document_id == doc_id


def test_wraps_api_errors(tmp_path, fake_client_factory):
    path = tmp_path / "x.txt"
    path.write_text("x")
    client = fake_client_factory(error=HaciendaApiError(403, "forbidden", "no documents:process"))

    loader = HaciendaLoader(file_path=path, client=client)
    with pytest.raises(HaciendaLoaderError, match="hacienda-engine request failed"):
        loader.load()


def test_unresolvable_mime_type_raises(tmp_path, fake_client_factory):
    path = tmp_path / "mystery.xyz123"
    path.write_text("?")

    loader = HaciendaLoader(file_path=path, client=fake_client_factory())
    with pytest.raises(HaciendaLoaderError, match="MIME type"):
        loader.load()


def test_document_id_accepts_str_and_uuid(tmp_path, fake_client_factory):
    path = tmp_path / "d.txt"
    path.write_text("d")
    doc_id = uuid4()
    result = make_result("d redacted", document_id=doc_id)
    client = fake_client_factory(response=make_response([result]))

    loader = HaciendaLoader(file_path=path, document_id=str(doc_id), client=client)
    docs = loader.load()

    # A string document_id is converted to a UUID before being sent — checked via the
    # public request/response surface, not the loader's private state.
    assert docs[0].metadata["document_id"] == str(doc_id)
    sent = client.documents.calls[0]
    assert sent.documents[0].document_id == doc_id
    assert isinstance(sent.documents[0].document_id, UUID)


def test_glob_filters_files_in_a_directory(tmp_path, fake_client_factory):
    (tmp_path / "a.pdf").write_text("a")
    (tmp_path / "b.txt").write_text("b")
    (tmp_path / "c.pdf").write_text("c")

    results = [make_result("a redacted"), make_result("c redacted")]
    client = fake_client_factory(response=make_response(results))

    loader = HaciendaLoader(file_path=tmp_path, glob="*.pdf", client=client)
    docs = loader.load()

    assert len(docs) == 2
    sent = client.documents.calls[0]
    sent_filenames = {doc.filename for doc in sent.documents}
    assert sent_filenames == {"a.pdf", "c.pdf"}


def test_context_manager_closes_owned_client(monkeypatch, tmp_path, fake_client_factory):
    path = tmp_path / "note.txt"
    path.write_text("hi")
    monkeypatch.setenv("HACIENDA_API_KEY", "test-key")

    # No client= passed, so the loader constructs (and must own/close) its own —
    # substitute a fake so closure is observable, since the real HaciendaClient
    # exposes no "closed" state to assert on.
    fake_client = fake_client_factory(response=make_response([make_result("hi redacted")]))
    monkeypatch.setattr(
        "langchain_hacienda.loader.HaciendaClient", lambda *a, **kw: fake_client
    )

    with HaciendaLoader(file_path=path) as loader:
        assert loader._owns_client is True
        assert fake_client.closed is False
        loader.load()

    assert fake_client.closed is True


def test_close_does_not_close_a_caller_supplied_client(tmp_path, fake_client_factory):
    path = tmp_path / "note.txt"
    path.write_text("hi")
    client = fake_client_factory(response=make_response([make_result("hi redacted")]))

    with HaciendaLoader(file_path=path, client=client) as loader:
        loader.load()

    assert client.closed is False
