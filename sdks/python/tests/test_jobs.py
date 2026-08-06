from __future__ import annotations

import base64

from hacienda_sdk import HaciendaClient
from hacienda_sdk._generated.models.document_input import DocumentInput
from hacienda_sdk._generated.models.process_documents_request import ProcessDocumentsRequest


def _submit_background_job(client: HaciendaClient) -> str:
    content = base64.b64encode(b"job listing fixture").decode("ascii")
    request = ProcessDocumentsRequest(
        documents=[DocumentInput(mime_type="text/plain", content_base64=content)]
    )
    return client.documents.process_documents_background(request).job_id


def test_get_job_result_matches_get_job_status(client: HaciendaClient) -> None:
    job_id = _submit_background_job(client)

    job = client.jobs.get_job(job_id)
    result = client.jobs.get_job_result(job_id)

    assert result.status == job.status


def test_list_jobs_includes_a_submitted_job(client: HaciendaClient) -> None:
    job_id = _submit_background_job(client)

    page = client.jobs.list_jobs()

    assert any(j.id == job_id for j in page.jobs)


def test_list_jobs_limit_is_respected(client: HaciendaClient) -> None:
    for _ in range(3):
        _submit_background_job(client)

    page = client.jobs.list_jobs(limit=1)

    assert len(page.jobs) == 1
