from __future__ import annotations

from types import SimpleNamespace
from typing import Any
from uuid import UUID

import pytest
from hacienda_sdk import HaciendaApiError


class FakeDocumentsNamespace:
    """Duck-typed stand-in for `hacienda_sdk.client._DocumentsNamespaceSync`.

    `HaciendaLoader` only reads attributes off whatever `process_documents` returns
    (never calls `.to_dict()` on it), so a `SimpleNamespace` response is enough to
    exercise the loader without a live server or the generated SDK's real response
    type.
    """

    def __init__(self, response: Any = None, error: HaciendaApiError | None = None) -> None:
        self.response = response
        self.error = error
        self.calls: list[Any] = []

    def process_documents(self, body: Any) -> Any:
        self.calls.append(body)
        if self.error is not None:
            raise self.error
        return self.response


class FakeHaciendaClient:
    def __init__(self, response: Any = None, error: HaciendaApiError | None = None) -> None:
        self.documents = FakeDocumentsNamespace(response=response, error=error)
        self.closed = False

    def close(self) -> None:
        self.closed = True


def make_entity(
    category: str = "EMAIL",
    start: int = 0,
    end: int = 10,
    confidence: float = 0.95,
    source: str = "regex",
) -> SimpleNamespace:
    return SimpleNamespace(
        category=category, start=start, end=end, confidence=confidence, source=source
    )


def make_result(
    content: str,
    entities: list[SimpleNamespace] | None = None,
    document_id: UUID | None = None,
    version_sequence: int | None = None,
) -> SimpleNamespace:
    from hacienda_sdk._generated.types import UNSET

    return SimpleNamespace(
        content=content,
        entities=entities or [],
        document_id=document_id if document_id is not None else UNSET,
        version_sequence=version_sequence if version_sequence is not None else UNSET,
    )


def make_response(
    documents: list[SimpleNamespace],
    processing_time_ms: int = 12,
    audit_chain_tip: str | None = "blake3:deadbeef",
) -> SimpleNamespace:
    from hacienda_sdk._generated.types import UNSET

    return SimpleNamespace(
        documents=documents,
        processing_time_ms=processing_time_ms,
        audit_chain_tip=audit_chain_tip if audit_chain_tip is not None else UNSET,
    )


@pytest.fixture
def fake_client_factory():
    return FakeHaciendaClient
