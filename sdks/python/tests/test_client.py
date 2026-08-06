from __future__ import annotations

from hacienda_sdk import HaciendaClient
from hacienda_sdk.client import Target


def test_health_and_info(client: HaciendaClient) -> None:
    health = client.info.get_health()
    assert health.status == "ok"

    info = client.info.get_info()
    assert info.name == "hacienda-api"


def test_unsupported_target_is_rejected() -> None:
    import pytest

    with pytest.raises(ValueError, match="cloud"):
        HaciendaClient(base_url="http://127.0.0.1:1", api_key="x", target="device")  # type: ignore[arg-type]


def test_target_literal_has_exactly_cloud() -> None:
    import typing

    assert typing.get_args(Target) == ("cloud",)


def test_context_manager_closes(client: HaciendaClient) -> None:
    import pytest

    with client:
        client.info.get_health()
    # httpx raises on reuse after close — proves __exit__ actually released the
    # underlying connection pool rather than being a no-op.
    with pytest.raises(RuntimeError, match="closed"):
        client.info.get_health()
