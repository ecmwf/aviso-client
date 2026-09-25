# (C) Copyright 2024- ECMWF and individual contributors.
#
# This software is licensed under the terms of the Apache Licence Version 2.0
# which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
# In applying this licence, ECMWF does not waive the privileges and immunities
# granted to it by virtue of its status as an intergovernmental organisation nor
# does it submit to any jurisdiction.

"""``Trigger.function`` with ``listen`` and ``listen_many``, and ``run()``."""

from __future__ import annotations

import logging
from collections import Counter
from functools import partial
from typing import Any, cast

import pyaviso
import pytest
from pyaviso import Trigger
from pytest_httpserver import HTTPServer

from .listen_server import START, entry, serve


@pytest.fixture
def client(httpserver: HTTPServer) -> pyaviso.AvisoClient:
    return pyaviso.AvisoClient(base_url=httpserver.url_for("/"))


def test_a_function_is_called_once_per_notification_with_its_arguments(
    httpserver: HTTPServer, client: pyaviso.AvisoClient
) -> None:
    serve(httpserver, {"mars": 3, "wave": 2})
    calls: list[tuple[str, str]] = []

    def record(n: pyaviso.Notification, tag: str) -> None:
        calls.append((tag, n.identifier["n"]))

    client.listen_many(
        {
            "surface": entry("mars", triggers=[Trigger.function(partial(record, tag="s"))]),
            "waves": entry("wave", triggers=[Trigger.function(partial(record, tag="w"))]),
        }
    ).run()
    assert sorted(calls) == [("s", "1"), ("s", "2"), ("s", "3"), ("w", "1"), ("w", "2")]


def test_a_failing_function_is_retried(httpserver: HTTPServer, client: pyaviso.AvisoClient) -> None:
    serve(httpserver, {"mars": 1})
    attempts = Counter[str]()

    def flaky(n: pyaviso.Notification) -> None:
        attempts["n"] += 1
        if attempts["n"] < 3:
            raise OSError("try again")

    client.listen_many({"s": entry("mars", triggers=[Trigger.function(flaky, retries=2)])}).run()
    assert attempts["n"] == 3


def test_a_required_function_that_fails_goes_to_on_error(
    httpserver: HTTPServer, client: pyaviso.AvisoClient
) -> None:
    serve(httpserver, {"mars": 3})

    def fails_on_two(n: pyaviso.Notification) -> None:
        if n.identifier["n"] == "2":
            raise ValueError("bad data")

    it = client.listen_many(
        {"s": entry("mars", triggers=[Trigger.function(fails_on_two)])}, on_error="continue"
    )
    delivered = [n.identifier["n"] for _, n in it]
    assert delivered == ["1", "3"], "the failed notification is skipped, the listener continues"
    [failure] = it.errors
    assert (failure.listener, failure.kind) == ("s", "trigger")
    assert isinstance(failure.error, pyaviso.TriggerError)
    assert failure.error.trigger_kind == "function"
    assert isinstance(failure.error.__cause__, ValueError)

    with pytest.raises(pyaviso.TriggerError, match="fails_on_two"):
        client.listen_many({"s": entry("mars", triggers=[Trigger.function(fails_on_two)])}).run()


def test_an_optional_function_that_fails_is_logged_and_the_notification_delivered(
    httpserver: HTTPServer, client: pyaviso.AvisoClient, caplog: pytest.LogCaptureFixture
) -> None:
    serve(httpserver, {"mars": 2})

    def always_fails(n: pyaviso.Notification) -> None:
        raise ValueError("ignored")

    with caplog.at_level(logging.WARNING, logger="pyaviso.listen"):
        it = client.listen_many(
            {"s": entry("mars", triggers=[Trigger.function(always_fails, required=False)])}
        )
        delivered = [n.identifier["n"] for _, n in it]
    assert delivered == ["1", "2"]
    assert it.errors == []
    assert "always_fails" in caplog.text


def test_the_sync_client_refuses_async_functions_before_opening(
    httpserver: HTTPServer, client: pyaviso.AvisoClient
) -> None:
    bodies = serve(httpserver, {"mars": 1})

    async def not_here(n: pyaviso.Notification) -> None:
        pass

    async def on_error(name: str, error: BaseException) -> None:
        pass

    for optional in (False, True):
        trigger = Trigger.function(not_here, required=not optional)
        with pytest.raises(TypeError, match=r"listener 's'.*needs\s+AsyncAvisoClient"):
            client.listen_many({"s": entry("mars", triggers=[trigger])})
        with pytest.raises(TypeError, match=r"needs\s+AsyncAvisoClient"):
            client.listen("mars", triggers=[trigger])
    with pytest.raises(TypeError, match="on_error is an async function"):
        client.listen_many({"s": entry("mars")}, on_error=on_error)
    assert bodies == [], "nothing may open when the arguments are refused"


def test_listen_calls_functions_too(httpserver: HTTPServer, client: pyaviso.AvisoClient) -> None:
    serve(httpserver, {"mars": 2})
    calls: list[str] = []
    it = client.listen(
        "mars",
        start_from=START,
        mode="replay_only",
        triggers=[Trigger.function(lambda n: calls.append(n.identifier["n"]))],
    )
    it.run()
    assert calls == ["1", "2"]


def test_trigger_function_settings() -> None:
    def fn(n: pyaviso.Notification) -> None:
        pass

    assert "function=test_trigger_function_settings.<locals>.fn," in repr(Trigger.function(fn))
    t = Trigger.function(fn, retries=1, label="mine")
    assert repr(t) == "Trigger(function=mine, retries=1, required=True)"
    assert "retries=3" in repr(t.retries(3))
    with pytest.raises(ValueError, match="timeout does not apply"):
        t.timeout(5)
    with pytest.raises(ValueError, match="fail_fast does not apply"):
        t.fail_fast(False)
    with pytest.raises(TypeError, match="callable"):
        Trigger.function(cast(Any, "not a function"))


def test_a_function_in_a_watch_request_passed_to_listen(
    httpserver: HTTPServer, client: pyaviso.AvisoClient
) -> None:
    serve(httpserver, {"mars": 2})
    calls: list[str] = []
    request = pyaviso.WatchRequest.replay_only("mars", START).with_triggers(
        [Trigger.function(lambda n: calls.append(n.identifier["n"]))]
    )
    client.listen(request=request).run()
    assert calls == ["1", "2"]


def test_ctrl_c_in_a_function_closes_every_listener(
    httpserver: HTTPServer, client: pyaviso.AvisoClient
) -> None:
    serve(httpserver, {"mars": 3})

    def interrupt(n: pyaviso.Notification) -> None:
        raise KeyboardInterrupt

    it = client.listen_many({"s": entry("mars", triggers=[Trigger.function(interrupt)])})
    with pytest.raises(KeyboardInterrupt):
        next(it)
    with pytest.raises(StopIteration):
        next(it)


def test_awaitables_from_plain_functions_always_stop(
    httpserver: HTTPServer, client: pyaviso.AvisoClient
) -> None:
    serve(httpserver, {"mars": 2, "broken": None})

    async def later() -> None:
        pass

    sneaky = Trigger.function(lambda n: later(), required=False)
    with pytest.raises(TypeError, match="returned an awaitable"):
        client.listen_many({"s": entry("mars", triggers=[sneaky])}, on_error="continue").run()

    with pytest.raises(TypeError, match="on_error returned an awaitable"):
        client.listen_many(
            {"good": entry("mars"), "bad": entry("broken")},
            on_error=lambda name, error: later(),
        ).run()


def test_a_function_trigger_error_has_every_declared_attribute(
    httpserver: HTTPServer, client: pyaviso.AvisoClient
) -> None:
    serve(httpserver, {"mars": 1})

    def fails(n: pyaviso.Notification) -> None:
        raise ValueError("bad")

    with pytest.raises(pyaviso.TriggerError) as caught:
        client.listen_many({"s": entry("mars", triggers=[Trigger.function(fails)])}).run()
    error = caught.value
    assert (error.trigger_kind, error.error_kind) == ("function", "raised")
    for name in (
        "path",
        "exit_code",
        "stderr_tail",
        "status",
        "body_tail",
        "reason",
        "timeout_seconds",
        "context",
        "field",
        "template_kind",
    ):
        assert getattr(error, name) is None, name
