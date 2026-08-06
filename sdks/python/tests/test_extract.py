from __future__ import annotations

import base64

from hacienda_sdk import HaciendaClient
from hacienda_sdk._generated.models.document_input import DocumentInput
from hacienda_sdk._generated.models.process_documents_request import ProcessDocumentsRequest


def test_process_documents_against_a_pii_disabled_server_returns_no_documents(
    client: HaciendaClient,
) -> None:
    # Known, already-documented pre-existing bug (CHANGELOG.md, Phase 13 Task 3):
    # `process_documents` zips extraction results with `result.pii`, which is an
    # empty Vec whenever no PII pipeline is configured (this test fixture's
    # default `hacienda serve`, per conftest.py) — so it silently returns
    # `documents: []` regardless of how many documents were submitted. Flagged
    # there as out of scope for that task, not fixed here either: this test
    # pins the actual current behavior so a future fix updates this assertion
    # deliberately, rather than the SDK silently start disagreeing with the API
    # it wraps. The processing_time_ms/audit_chain_tip envelope fields are
    # still present, which is what the round-trip below (test_jobs.py) relies
    # on instead of document content.
    content = base64.b64encode(b"hello world, no PII here").decode("ascii")
    request = ProcessDocumentsRequest(
        documents=[
            DocumentInput(mime_type="text/plain", content_base64=content, filename="note.txt")
        ]
    )

    result = client.documents.process_documents(request)

    assert result.documents == []


def test_process_documents_background_returns_a_pollable_job(client: HaciendaClient) -> None:
    content = base64.b64encode(b"background job body").decode("ascii")
    request = ProcessDocumentsRequest(
        documents=[DocumentInput(mime_type="text/plain", content_base64=content)]
    )

    accepted = client.documents.process_documents_background(request)
    assert accepted.job_id

    job = client.jobs.get_job(accepted.job_id)
    assert job.id == accepted.job_id
