from __future__ import annotations

from dataclasses import dataclass

import pytest

from hacienda_sdk.errors import HaciendaApiError, _unwrap_json


@dataclass
class FakeResponse:
    status_code: int
    content: bytes
    parsed: object = None


class TestUnwrapJson:
    def test_returns_parsed_when_present(self) -> None:
        response = FakeResponse(status_code=200, content=b"{}", parsed={"ok": True})
        assert _unwrap_json(response) == {"ok": True}

    def test_falls_back_to_parsing_content_when_parsed_is_none(self) -> None:
        response = FakeResponse(status_code=200, content=b'{"chunks": []}', parsed=None)
        assert _unwrap_json(response) == {"chunks": []}

    def test_returns_none_for_empty_body(self) -> None:
        response = FakeResponse(status_code=204, content=b"", parsed=None)
        assert _unwrap_json(response) is None

    def test_raises_hacienda_api_error_on_malformed_json(self) -> None:
        response = FakeResponse(status_code=200, content=b"not json", parsed=None)
        with pytest.raises(HaciendaApiError, match="could not parse response body as JSON"):
            _unwrap_json(response)

    def test_raises_hacienda_api_error_on_undecodable_bytes(self) -> None:
        response = FakeResponse(status_code=200, content=b"\xff\xfe\x00", parsed=None)
        with pytest.raises(HaciendaApiError, match="could not parse response body as JSON"):
            _unwrap_json(response)

    def test_error_status_still_routes_through_unwrap(self) -> None:
        response = FakeResponse(
            status_code=403,
            content=b'{"error": {"code": "forbidden", "message": "no rag:write"}}',
            parsed=None,
        )
        with pytest.raises(HaciendaApiError) as exc_info:
            _unwrap_json(response)
        assert exc_info.value.status_code == 403
        assert exc_info.value.code == "forbidden"
