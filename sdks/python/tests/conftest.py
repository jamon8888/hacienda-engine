"""Starts a real `hacienda serve` instance for the test session.

Per `testing-anti-patterns` (never mock what you don't own): every test in this
suite exercises `HaciendaClient`/`AsyncHaciendaClient` against a live server
built from the same checkout, not a mocked transport.
"""

from __future__ import annotations

import socket
import subprocess
import time
from collections.abc import Iterator
from pathlib import Path

import pytest

from hacienda_sdk import AsyncHaciendaClient, HaciendaClient

REPO_ROOT = Path(__file__).resolve().parents[3]


def _free_port() -> int:
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as s:
        s.bind(("127.0.0.1", 0))
        return s.getsockname()[1]


@pytest.fixture(scope="session")
def hacienda_base_url() -> Iterator[str]:
    subprocess.run(["cargo", "build", "-p", "hacienda-cli"], cwd=REPO_ROOT, check=True)
    port = _free_port()
    base_url = f"http://127.0.0.1:{port}"
    proc = subprocess.Popen(
        [str(REPO_ROOT / "target" / "debug" / "hacienda"), "serve", "--bind", f"127.0.0.1:{port}"],
        cwd=REPO_ROOT,
    )
    try:
        for _ in range(50):
            try:
                import httpx

                if httpx.get(f"{base_url}/health", timeout=1.0).status_code == 200:
                    break
            except httpx.TransportError:
                pass
            time.sleep(0.2)
        else:
            raise RuntimeError("hacienda serve did not become ready in time")
        yield base_url
    finally:
        proc.terminate()
        proc.wait(timeout=5)


@pytest.fixture
def client(hacienda_base_url: str) -> HaciendaClient:
    return HaciendaClient(base_url=hacienda_base_url, api_key="test")


@pytest.fixture
def async_client(hacienda_base_url: str) -> AsyncHaciendaClient:
    return AsyncHaciendaClient(base_url=hacienda_base_url, api_key="test")
