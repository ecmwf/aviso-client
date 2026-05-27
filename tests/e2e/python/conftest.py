"""End-to-end pytest fixtures backed by the docker-compose stack at tests/e2e/.

The fixtures here run against the real aviso-server + auth-o-tron + nats stack.
Each pytest session brings the stack up once via ``tests/e2e/shared/stack_up.sh``
and reuses it across every test.

By default the stack is LEFT RUNNING after the session ends so successive ``uv run
pytest`` invocations skip the docker startup cost. Set ``AVISO_E2E_TEARDOWN=1`` to
tear down (``docker compose down -v``) when the session finishes.

On any test failure during the session, ``docker compose logs`` are captured to
``tests/e2e/last-failure.log`` for debugging.
"""

from __future__ import annotations

import os
import subprocess
from collections.abc import Iterator
from pathlib import Path

import aviso
import pytest

E2E_DIR = Path(__file__).resolve().parents[1]
COMPOSE_FILE = E2E_DIR / "docker-compose.yml"
STACK_UP = E2E_DIR / "shared" / "stack_up.sh"
LAST_FAILURE_LOG = E2E_DIR / "last-failure.log"

BASE_URL = os.environ.get("AVISO_E2E_BASE_URL", "http://localhost:8000")

ADMIN_USERNAME = "admin-user"
ADMIN_PASSWORD = "admin-pass"
READER_USERNAME = "reader-user"
READER_PASSWORD = "reader-pass"
PRODUCER_USERNAME = "producer-user"
PRODUCER_PASSWORD = "producer-pass"


@pytest.fixture(scope="session", autouse=True)
def e2e_stack() -> Iterator[None]:
    """Brings the e2e stack up via stack_up.sh, leaves it running by default."""
    subprocess.run(["bash", str(STACK_UP)], check=True)
    yield
    if os.environ.get("AVISO_E2E_TEARDOWN") == "1":
        subprocess.run(
            ["docker", "compose", "-f", str(COMPOSE_FILE), "down", "-v"],
            check=False,
        )


@pytest.fixture
def base_url() -> str:
    return BASE_URL


@pytest.fixture
def producer_auth() -> aviso.Basic:
    return aviso.Basic(PRODUCER_USERNAME, PRODUCER_PASSWORD)


@pytest.fixture
def reader_auth() -> aviso.Basic:
    return aviso.Basic(READER_USERNAME, READER_PASSWORD)


@pytest.fixture
def admin_auth() -> aviso.Basic:
    return aviso.Basic(ADMIN_USERNAME, ADMIN_PASSWORD)


@pytest.fixture
def producer_client(base_url: str, producer_auth: aviso.Basic) -> Iterator[aviso.AvisoClient]:
    with aviso.AvisoClient(base_url=base_url, auth=producer_auth) as client:
        yield client


@pytest.fixture
def reader_client(base_url: str, reader_auth: aviso.Basic) -> Iterator[aviso.AvisoClient]:
    with aviso.AvisoClient(base_url=base_url, auth=reader_auth) as client:
        yield client


def pytest_sessionfinish(session: pytest.Session, exitstatus: int) -> None:
    """Capture docker compose logs on any failure for post-mortem."""
    if exitstatus == 0:
        return
    try:
        with LAST_FAILURE_LOG.open("w", encoding="utf-8") as out:
            subprocess.run(
                ["docker", "compose", "-f", str(COMPOSE_FILE), "logs"],
                stdout=out,
                stderr=subprocess.STDOUT,
                check=False,
            )
    except OSError:
        pass
