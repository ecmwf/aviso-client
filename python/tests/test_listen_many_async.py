# (C) Copyright 2024- ECMWF and individual contributors.
#
# This software is licensed under the terms of the Apache Licence Version 2.0
# which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
# In applying this licence, ECMWF does not waive the privileges and immunities
# granted to it by virtue of its status as an intergovernmental organisation nor
# does it submit to any jurisdiction.

"""``listen_many``, function triggers and closing on ``AsyncAvisoClient``."""

from __future__ import annotations

import asyncio
from collections import Counter

import pyaviso
import pytest
from pyaviso import Trigger
from pytest_httpserver import HTTPServer

from .listen_server import START, entry, quiet, serve


def test_async_listen_many_awaits_async_functions_and_on_error(httpserver: HTTPServer) -> None:
    serve(httpserver, {"mars": 3, "wave": 2, "broken": None})
    calls: list[str] = []
    asked: list[str] = []

    async def record(n: pyaviso.Notification) -> None:
        await asyncio.sleep(0)
        calls.append(n.identifier["n"])

    async def report(name: str, error: BaseException) -> None:
        await asyncio.sleep(0)
        asked.append(name)

    async def main() -> Counter[str]:
        client = pyaviso.AsyncAvisoClient(base_url=httpserver.url_for("/"))
        seen: Counter[str] = Counter()
        async with client.listen_many(
            {
                "surface": entry("mars", triggers=[Trigger.function(record)]),
                "waves": entry("wave"),
                "bad": entry("broken"),
            },
            on_error=report,
        ) as it:
            async for name, _ in it:
                seen[name] += 1
        return seen

    seen = asyncio.run(main())
    assert seen == {"surface": 3, "waves": 2}
    assert calls == ["1", "2", "3"]
    assert asked == ["bad"]


def test_async_all_failed_raises(httpserver: HTTPServer) -> None:
    serve(httpserver, {"broken": None, "gone": None})

    async def main() -> None:
        client = pyaviso.AsyncAvisoClient(base_url=httpserver.url_for("/"))
        it = client.listen_many({"a": entry("broken"), "b": entry("gone")}, on_error="continue")
        with pytest.raises(pyaviso.AvisoError, match="every listener failed"):
            await it.run()
        with pytest.raises(StopAsyncIteration):
            await it.__anext__()

    asyncio.run(main())


def test_aclose_returns_while_a_read_waits(httpserver: HTTPServer) -> None:
    quiet(httpserver)

    async def main() -> None:
        client = pyaviso.AsyncAvisoClient(base_url=httpserver.url_for("/"))
        for it in (
            client.listen_many({"q": {"event_type": "mars"}}),
            client.listen("mars"),
        ):
            reading = asyncio.ensure_future(it.__anext__())
            await asyncio.sleep(0.3)  # the read is now waiting on the stream
            await asyncio.wait_for(it.aclose(), 5)
            with pytest.raises(StopAsyncIteration):
                await asyncio.wait_for(reading, 5)

    asyncio.run(main())


def test_cancelling_run_closes_the_listener(httpserver: HTTPServer) -> None:
    quiet(httpserver)

    async def main() -> None:
        client = pyaviso.AsyncAvisoClient(base_url=httpserver.url_for("/"))
        for it in (
            client.listen_many({"q": {"event_type": "mars"}}),
            client.listen("mars"),
        ):
            task = asyncio.ensure_future(it.run())
            await asyncio.sleep(0.3)
            task.cancel()
            with pytest.raises(asyncio.CancelledError):
                await task
            # Closed: the next read ends at once instead of waiting.
            with pytest.raises(StopAsyncIteration):
                await asyncio.wait_for(it.__anext__(), 5)

    asyncio.run(main())


def test_async_errors_and_failing_async_functions(httpserver: HTTPServer) -> None:
    serve(httpserver, {"mars": 2, "broken": None})

    async def boom(n: pyaviso.Notification) -> None:
        raise ValueError("bad")

    async def main() -> None:
        client = pyaviso.AsyncAvisoClient(base_url=httpserver.url_for("/"))
        it = client.listen_many(
            {"s": entry("mars", triggers=[Trigger.function(boom)]), "bad": entry("broken")},
            on_error="continue",
        )
        await it.run()
        assert sorted((f.listener, f.kind) for f in it.errors) == [
            ("bad", "listener"),
            ("s", "trigger"),
            ("s", "trigger"),
        ]
        with pytest.raises(pyaviso.TriggerError, match="boom"):
            await client.listen(
                "mars", start_from=START, mode="replay_only", triggers=[Trigger.function(boom)]
            ).run()

    asyncio.run(main())


def test_async_run_and_listen_with_a_function(httpserver: HTTPServer) -> None:
    serve(httpserver, {"mars": 2})
    calls: list[str] = []

    async def main() -> None:
        client = pyaviso.AsyncAvisoClient(base_url=httpserver.url_for("/"))
        await client.listen_many({"s": entry("mars")}).run()
        await client.listen(
            "mars",
            start_from=START,
            mode="replay_only",
            triggers=[Trigger.function(lambda n: calls.append(n.identifier["n"]))],
        ).run()
        await client.listen("mars", start_from=START, mode="replay_only").run()

    asyncio.run(main())
    assert calls == ["1", "2"]


def test_a_timeout_around_one_read_leaves_the_listeners_open(httpserver: HTTPServer) -> None:
    quiet(httpserver)

    async def main() -> None:
        client = pyaviso.AsyncAvisoClient(base_url=httpserver.url_for("/"))
        for it in (client.listen_many({"q": {"event_type": "mars"}}), client.listen("mars")):
            with pytest.raises(asyncio.TimeoutError):
                await asyncio.wait_for(it.__anext__(), 0.3)
            # Still open: a second read waits again instead of ending.
            with pytest.raises(asyncio.TimeoutError):
                await asyncio.wait_for(it.__anext__(), 0.3)
            await it.aclose()

    asyncio.run(main())
