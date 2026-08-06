from __future__ import annotations

import pytest

from hacienda_sdk import HaciendaApiError, HaciendaClient


def test_whoami_reports_capabilities_when_auth_is_disabled(client: HaciendaClient) -> None:
    # `hacienda serve` in this test fixture runs with auth disabled (loopback
    # only), so every caller is `Caller::Trusted` — see
    # `hacienda-api/src/handlers/auth.rs::whoami`'s doc comment: Trusted maps to
    # every capability, and `principal_id` is `None`.
    response = client.whoami()

    assert response.principal_id is None
    assert "documents:process" in response.capabilities
    assert response.capabilities == sorted(response.capabilities)


def test_whoami_matches_direct_namespace_call(client: HaciendaClient) -> None:
    assert client.whoami() == client.auth.get_whoami()


def test_invalid_request_raises_hacienda_api_error(client: HaciendaClient) -> None:
    # /v1/pii/reveal against this fixture's default config (no PII pipeline
    # configured) deterministically 400s — either "PII detection is not
    # enabled" or a malformed-token rejection, both invalid_request per
    # hacienda-api/src/error.rs. Either way, this must surface as a
    # HaciendaApiError, not a silently swallowed None (see errors.py's module
    # doc for why that distinction is the point of this test).
    from hacienda_sdk._generated.models.reveal_token_request import RevealTokenRequest

    with pytest.raises(HaciendaApiError) as exc_info:
        client.pii.reveal_token(RevealTokenRequest(token="not-a-real-token"))

    assert exc_info.value.status_code == 400
    assert exc_info.value.code == "invalid_request"
