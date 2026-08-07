from __future__ import annotations

import base64

from hacienda_sdk import HaciendaClient
from hacienda_sdk._generated.models.document_input import DocumentInput
from hacienda_sdk._generated.models.process_documents_request import ProcessDocumentsRequest


def test_process_documents_against_a_pii_disabled_server_still_returns_extraction(
    client: HaciendaClient,
) -> None:
    # `process_documents` used to zip extraction results with `result.pii`, which is
    # an empty Vec whenever no PII pipeline is configured (this test fixture's
    # default `hacienda serve`, per conftest.py) — the zip silently dropped every
    # document regardless of how many were submitted. Fixed server-side; this
    # asserts the corrected behavior: the document is still returned, just with no
    # entities.
    content = base64.b64encode(b"hello world, no PII here").decode("ascii")
    request = ProcessDocumentsRequest(
        documents=[
            DocumentInput(mime_type="text/plain", content_base64=content, filename="note.txt")
        ]
    )

    result = client.documents.process_documents(request)

    assert len(result.documents) == 1
    assert result.documents[0].entities == []


def test_process_documents_background_returns_a_pollable_job(client: HaciendaClient) -> None:
    content = base64.b64encode(b"background job body").decode("ascii")
    request = ProcessDocumentsRequest(
        documents=[DocumentInput(mime_type="text/plain", content_base64=content)]
    )

    accepted = client.documents.process_documents_background(request)
    assert accepted.job_id

    job = client.jobs.get_job(accepted.job_id)
    assert job.id == accepted.job_id
