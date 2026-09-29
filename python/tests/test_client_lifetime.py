# (C) Copyright 2024- ECMWF and individual contributors.
#
# This software is licensed under the terms of the Apache Licence Version 2.0
# which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
# In applying this licence, ECMWF does not waive the privileges and immunities
# granted to it by virtue of its status as an intergovernmental organisation nor
# does it submit to any jurisdiction.

"""A stream keeps working after its client object is gone.

``pyaviso.AvisoClient(...).listen(...)`` leaves no reference to the client, so
Python frees it as soon as ``listen`` returns. The stream must still deliver
every notification, not end early as if the replay were complete.

The server holds each reply until the test has freed the client, so nothing
can be delivered before that point. If the test never gets there, the server
refuses the request with a 400, which ends the stream with an error, and the
test also fails on the gate's record.
"""

from __future__ import annotations

import asyncio
import gc
import json
import re
import threading
from pathlib import Path
from typing import TypeVar

import pyaviso
from pyaviso import Trigger
from pytest_httpserver import HTTPServer
from werkzeug import Request, Response

from .listen_server import START, entry, replay

T = TypeVar("T")

# How long the server waits for the test to free the client.
_GATE_TIMEOUT = 10.0


class _Gate:
    """Holds the server's replies until the test opens it."""

    def __init__(self) -> None:
        self._event = threading.Event()
        self.expired = False

    def wait(self) -> bool:
        if self._event.wait(_GATE_TIMEOUT):
            return True
        self.expired = True
        return False

    def release(self, opened: T) -> T:
        """Frees whatever was left unreferenced, then lets the server reply."""
        gc.collect()
        self._event.set()
        return opened


def _serve_after(httpserver: HTTPServer) -> _Gate:
    """Answers with a replay of three notifications once the gate is open.
    A gate that stays closed gets a 400, which the client does not retry."""
    gate = _Gate()

    def handler(request: Request) -> Response:
        if not gate.wait():
            return Response("gate never opened", status=400)
        event_type = json.loads(request.data)["event_type"]
        return Response(replay(event_type, 3), content_type="text/event-stream")

    httpserver.expect_request(
        re.compile("^/api/v1/(watch|replay)$"), method="POST"
    ).respond_with_handler(handler)
    return gate


def test_listen_outlives_its_client(httpserver: HTTPServer) -> None:
    gate = _serve_after(httpserver)
    stream = gate.release(
        pyaviso.AvisoClient(base_url=httpserver.url_for("/")).listen(
            "mars", start_from=START, mode="replay_only"
        ),
    )
    with stream:
        assert [n.identifier["n"] for n in stream] == ["1", "2", "3"]
    assert not gate.expired


def test_listen_many_outlives_its_client(httpserver: HTTPServer) -> None:
    gate = _serve_after(httpserver)
    stream = gate.release(
        pyaviso.AvisoClient(base_url=httpserver.url_for("/")).listen_many(
            {"surface": entry("mars")}
        ),
    )
    with stream:
        assert [n.identifier["n"] for _, n in stream] == ["1", "2", "3"]
    assert not gate.expired


def test_async_listen_outlives_its_client(httpserver: HTTPServer) -> None:
    gate = _serve_after(httpserver)

    async def main() -> list[str]:
        stream = gate.release(
            pyaviso.AsyncAvisoClient(base_url=httpserver.url_for("/")).listen(
                "mars", start_from=START, mode="replay_only"
            ),
        )
        async with stream:
            seen = [n.identifier["n"] async for n in stream]
        return seen

    assert asyncio.run(main()) == ["1", "2", "3"]
    assert not gate.expired


def test_async_listen_many_outlives_its_client(httpserver: HTTPServer) -> None:
    gate = _serve_after(httpserver)

    async def main() -> list[str]:
        stream = gate.release(
            pyaviso.AsyncAvisoClient(base_url=httpserver.url_for("/")).listen_many(
                {"surface": entry("mars")}
            ),
        )
        async with stream:
            seen = [n.identifier["n"] async for _, n in stream]
        return seen

    assert asyncio.run(main()) == ["1", "2", "3"]
    assert not gate.expired


def test_async_run_outlives_its_iterator(httpserver: HTTPServer, tmp_path: Path) -> None:
    """The awaitable from ``run()`` may be all that is left of the stream."""
    gate = _serve_after(httpserver)
    log = tmp_path / "notifications.jsonl"

    async def main() -> None:
        await gate.release(
            pyaviso.AsyncAvisoClient(base_url=httpserver.url_for("/"))
            .listen("mars", start_from=START, mode="replay_only", triggers=[Trigger.log(log)])
            .run(),
        )

    asyncio.run(main())
    assert len(log.read_text(encoding="utf-8").splitlines()) == 3
    assert not gate.expired


def test_async_next_outlives_its_iterator(httpserver: HTTPServer) -> None:
    """The awaitable from ``anext()`` may be all that is left of the stream."""
    gate = _serve_after(httpserver)

    async def main() -> pyaviso.Notification:
        return await gate.release(
            anext(
                pyaviso.AsyncAvisoClient(base_url=httpserver.url_for("/")).listen(
                    "mars", start_from=START, mode="replay_only"
                )
            ),
        )

    assert asyncio.run(main()).identifier["n"] == "1"
    assert not gate.expired
