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
