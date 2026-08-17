# hacienda-sdk

Official Python client for the [hacienda-engine](https://github.com/jamon8888/hacienda-engine)
PII redaction and compliance API.

```python
from hacienda_sdk import HaciendaClient

client = HaciendaClient(base_url="https://your-hacienda-instance", api_key="hcd_live_...")
result = client.pii.scan_text("Contact Jane Doe at jane@example.com.")
for entity in result.entities:
    print(entity.category, entity.start, entity.end)
```

An async client is available as `AsyncHaciendaClient`, mirroring every method as a
coroutine.

## Async jobs

`documents.process_documents_background(...)` returns a job id immediately; poll it
yourself via `jobs.get_job_result(...)`, or use the `extract_and_wait(...)` /
`wait_for_job(...)` / `wait_for_jobs(...)` convenience methods on the client, which poll
for you and raise `JobTimeoutError` if the job hasn't finished within `timeout` seconds:

```python
result = client.extract_and_wait(body, poll_interval=1.0, timeout=300.0)
```

## Retries

Both clients retry a request on `429`, `502`, `503`, and `504` (configurable via
`retry_statuses`), honoring `Retry-After` when the server sends it and falling back to
exponential backoff with jitter otherwise. Only idempotent HTTP methods (`GET`, `HEAD`,
`OPTIONS`, `DELETE`) are replayed — the API has no idempotency-key mechanism for `POST`,
so a retried `POST` (e.g. `process_documents`, `confirm_upload`) could create a duplicate
resource if the origin already processed the first attempt. `POST` responses are never
replayed, even on a retryable status.

## Development

See [`../README.md`](../README.md) for the shared `sdks/` layout. From this
directory:

```sh
uv sync
bash scripts/generate.sh   # regenerates src/hacienda_sdk/_generated/ (gitignored)
uv run pytest
uv run ruff check src tests
uv run mypy src
```
