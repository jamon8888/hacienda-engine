"""SDK-level exceptions.

Every wrapper method routes through `_detailed` generated functions (never the
plain `sync`/`asyncio` convenience functions) and `_unwrap`/`_unwrap_async` below,
specifically because the generated convenience functions collapse *every*
non-2xx status — including the documented 401/403/404/400 cases, whose OpenAPI
responses carry no typed body — into `None`. That makes an error indistinguishable
from a legitimately empty success. Routing through `_detailed` and raising here
means a caller can `except HaciendaApiError` and inspect `.status_code`/`.code`
instead of silently receiving `None`.
"""

from __future__ import annotations

import json
from typing import Any


class HaciendaApiError(Exception):
    """Raised for any non-2xx response from the hacienda API.

    Mirrors the wire error envelope hacienda-api sends on every error response
    (`{"error": {"code": "...", "message": "..."}}`, see that crate's `error.rs`):
    `code` is the machine-readable snake_case string (`"invalid_request"`,
    `"forbidden"`, ...), `message` is the client-safe sentence. Both fall back to
    `None`/a generic message if the body could not be parsed as that envelope —
    still a `HaciendaApiError`, just without the structured detail.
    """

    def __init__(self, status_code: int, code: str | None, message: str) -> None:
        self.status_code = status_code
        self.code = code
        self.message = message
        super().__init__(f"HTTP {status_code} ({code}): {message}")


def _unwrap(response: Any) -> Any:
    """Return `response.parsed` on 2xx, raise `HaciendaApiError` otherwise.

    `response` is a generated `Response[T]` (has `.status_code`, `.content`,
    `.parsed`) — untyped here to avoid importing the codegen output from this
    hand-written module (`_generated/` is regenerated and gitignored; nothing
    outside `client.py` should import it).
    """
    status = int(response.status_code)
    if 200 <= status < 300:
        return response.parsed

    code: str | None = None
    message = f"request failed with status {status}"
    try:
        body = json.loads(response.content)
        error = body.get("error") if isinstance(body, dict) else None
        if isinstance(error, dict):
            code = error.get("code")
            message = error.get("message", message)
    except (json.JSONDecodeError, UnicodeDecodeError):
        pass

    raise HaciendaApiError(status_code=status, code=code, message=message)
