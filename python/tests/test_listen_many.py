# (C) Copyright 2024- ECMWF and individual contributors.
#
# This software is licensed under the terms of the Apache Licence Version 2.0
# which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
# In applying this licence, ECMWF does not waive the privileges and immunities
# granted to it by virtue of its status as an intergovernmental organisation nor
# does it submit to any jurisdiction.

"""``listen_many`` on the synchronous client: delivery, ``on_error``, argument
checks and stopping."""

from __future__ import annotations

import json
import logging
import pathlib
import types
from collections import Counter
from typing import Any

import pyaviso
import pytest
from pyaviso import Trigger
from pytest_httpserver import HTTPServer

from .listen_server import START, entry, serve


@pytest.fixture
def client(httpserver: HTTPServer) -> pyaviso.AvisoClient:
    return pyaviso.AvisoClient(base_url=httpserver.url_for("/"))


def test_the_loop_yields_names_and_ends_when_every_listener_ends(
    httpserver: HTTPServer, client: pyaviso.AvisoClient
) -> None:
    serve(httpserver, {"mars": 3, "wave": 2})
    seen: Counter[str] = Counter()
    with client.listen_many({"surface": entry("mars"), "waves": entry("wave")}) as it:
        for name, n in it:
            assert isinstance(n, pyaviso.Notification)
            seen[name] += 1
    assert seen == {"surface": 3, "waves": 2}
    assert it.errors == []


def test_shared_settings_apply_where_an_entry_sets_none(
    httpserver: HTTPServer, client: pyaviso.AvisoClient
) -> None:
    bodies = serve(httpserver, {"mars": 1, "wave": 1})
    listeners = {
        "shared": {"event_type": "mars"},
        "own": {"event_type": "wave", "start_from": 5, "mode": "replay_only"},
    }
    client.listen_many(listeners, start_from=START, mode="replay_only").run()
    by_type = {b["event_type"]: b for b in bodies}
    assert by_type["mars"]["from_date"] == START
    assert by_type["wave"]["from_id"] == "6"  # start_from=5 means after sequence 5


def test_a_watch_request_is_an_entry_too(
    httpserver: HTTPServer, client: pyaviso.AvisoClient
) -> None:
    serve(httpserver, {"mars": 2})
    calls: list[str] = []
    request = pyaviso.WatchRequest.replay_only("mars", START).with_triggers(
        [Trigger.function(lambda n: calls.append(n.identifier["n"]))]
    )
    client.listen_many({"req": request}).run()
    assert calls == ["1", "2"]


def test_raise_stops_everything_and_names_the_listener(
    httpserver: HTTPServer, client: pyaviso.AvisoClient
) -> None:
    serve(httpserver, {"mars": 50, "broken": None})
    with pytest.raises(pyaviso.HttpError) as caught:
        client.listen_many({"good": entry("mars"), "bad": entry("broken")}).run()
    error = caught.value
    assert error.listener == "bad"
    assert str(error).startswith("listener 'bad': ")
    assert error.status == 400


def test_continue_drops_the_failed_listener_and_keeps_the_rest(
    httpserver: HTTPServer, client: pyaviso.AvisoClient, caplog: pytest.LogCaptureFixture
) -> None:
    serve(httpserver, {"mars": 3, "broken": None})
    seen: Counter[str] = Counter()
    with caplog.at_level(logging.WARNING, logger="pyaviso.listen"):
        it = client.listen_many(
            {"good": entry("mars"), "bad": entry("broken")}, on_error="continue"
        )
        for name, _ in it:
            seen[name] += 1
    assert seen == {"good": 3}
    [failure] = it.errors
    assert (failure.listener, failure.kind) == ("bad", "listener")
    assert isinstance(failure.error, pyaviso.HttpError)
    assert "listener 'bad'" in caplog.text


def test_an_on_error_function_is_asked_and_can_stop_everything(
    httpserver: HTTPServer, client: pyaviso.AvisoClient
) -> None:
    serve(httpserver, {"mars": 3, "broken": None})
    asked: list[tuple[str, str]] = []

    def keep_going(name: str, error: BaseException) -> None:
        asked.append((name, type(error).__name__))

    client.listen_many({"good": entry("mars"), "bad": entry("broken")}, on_error=keep_going).run()
    assert asked == [("bad", "HttpError")]

    def stop(name: str, error: BaseException) -> None:
        raise RuntimeError(f"stop because of {name}")

    with pytest.raises(RuntimeError, match="stop because of bad"):
        client.listen_many({"good": entry("mars"), "bad": entry("broken")}, on_error=stop).run()


def test_when_every_listener_fails_the_loop_raises(
    httpserver: HTTPServer, client: pyaviso.AvisoClient
) -> None:
    serve(httpserver, {"broken": None, "gone": None})
    it = client.listen_many({"a": entry("broken"), "b": entry("gone")}, on_error="continue")
    with pytest.raises(pyaviso.AvisoError, match="every listener failed") as caught:
        it.run()
    failures = caught.value.failures
    assert failures is not None
    assert sorted(f.listener for f in failures) == ["a", "b"]


def test_one_listener_ending_normally_is_not_every_listener_failing(
    httpserver: HTTPServer, client: pyaviso.AvisoClient
) -> None:
    serve(httpserver, {"mars": 0, "broken": None})
    it = client.listen_many({"fine": entry("mars"), "bad": entry("broken")}, on_error="continue")
    it.run()
    assert [f.listener for f in it.errors] == ["bad"]


def test_after_the_loop_stops_next_just_stops(
    httpserver: HTTPServer, client: pyaviso.AvisoClient
) -> None:
    serve(httpserver, {"broken": None, "gone": None})
    it = client.listen_many({"a": entry("broken")})
    with pytest.raises(pyaviso.HttpError):
        next(it)
    with pytest.raises(StopIteration):
        next(it)

    it = client.listen_many({"a": entry("broken"), "b": entry("gone")}, on_error="continue")
    with pytest.raises(pyaviso.AvisoError, match="every listener failed"):
        next(it)
    with pytest.raises(StopIteration):
        next(it)


def test_an_on_error_function_is_not_doubled_by_a_warning(
    httpserver: HTTPServer, client: pyaviso.AvisoClient, caplog: pytest.LogCaptureFixture
) -> None:
    serve(httpserver, {"mars": 1, "broken": None})
    with caplog.at_level(logging.WARNING, logger="pyaviso.listen"):
        client.listen_many(
            {"good": entry("mars"), "bad": entry("broken")}, on_error=lambda name, error: None
        ).run()
    assert "listener 'bad'" not in caplog.text


def test_mode_may_be_a_watch_mode_and_entries_any_mapping(
    httpserver: HTTPServer, client: pyaviso.AvisoClient
) -> None:

    bodies = serve(httpserver, {"mars": 1, "wave": 1})
    listeners = types.MappingProxyType(
        {
            "own": types.MappingProxyType(
                {"event_type": "mars", "start_from": START, "mode": pyaviso.WatchMode.REPLAY_ONLY}
            ),
            "shared": {"event_type": "wave"},
        }
    )
    client.listen_many(listeners, start_from=START, mode=pyaviso.WatchMode.REPLAY_ONLY).run()
    assert sorted(b["event_type"] for b in bodies) == ["mars", "wave"]


@pytest.mark.parametrize(
    ("listeners", "kwargs", "error", "match"),
    [
        ({}, {}, ValueError, "at least one listener"),
        ([("a", {"event_type": "mars"})], {}, TypeError, "dict mapping a name"),
        ({"a": {"event_type": "mars", "filtre": {}}}, {}, ValueError, "unknown key 'filtre'"),
        ({"a": {"filter": {}}}, {}, ValueError, "listener 'a' needs an event_type"),
        ({"a": "mars"}, {}, TypeError, "dict of listen\\(\\) arguments or a WatchRequest"),
        ({1: {"event_type": "mars"}}, {}, TypeError, "names must be strings"),
        ({"": {"event_type": "mars"}}, {}, ValueError, "must not be empty"),
        # Shared options are checked even when no entry uses them.
        (
            {"a": {"event_type": "mars", "start_from": 5, "mode": "replay_only"}},
            {"mode": "bogus"},
            ValueError,
            "mode must be",
        ),
        (
            {"a": {"event_type": "mars", "start_from": 5, "mode": "replay_only"}},
            {"start_from": 1.5},
            TypeError,
            "start_from",
        ),
        ({"a": {"event_type": "mars"}}, {"on_error": "ignore"}, ValueError, "on_error"),
        ({"a": {"event_type": "mars"}}, {"on_error": 3}, TypeError, "on_error"),
        # An invalid listen() argument raises what listen() raises, named.
        (
            {"a": {"event_type": "mars", "mode": "replay_only"}},
            {},
            pyaviso.AvisoError,
            "listener 'a': replay_only mode requires start_from",
        ),
    ],
)
def test_bad_arguments_are_refused_before_anything_opens(
    httpserver: HTTPServer,
    client: pyaviso.AvisoClient,
    listeners: Any,
    kwargs: dict[str, Any],
    error: type[BaseException],
    match: str,
) -> None:
    bodies = serve(httpserver, {"mars": 1})
    with pytest.raises(error, match=match):
        client.listen_many(listeners, **kwargs)
    assert bodies == []


def test_each_listener_keeps_its_own_saved_position(
    httpserver: HTTPServer, tmp_path: pathlib.Path
) -> None:
    serve(httpserver, {"mars": 3, "wave": 2})
    state = tmp_path / "state.json"
    client = pyaviso.AvisoClient(
        base_url=httpserver.url_for("/"),
        state_store=pyaviso.JsonFileStore(state),
        flush_cursor_on_exit=True,
    )
    client.listen_many({"a": entry("mars"), "b": entry("wave")}).run()
    saved = json.loads(state.read_text(encoding="utf-8"))["checkpoints"].values()
    assert sorted(c["last_event_id"] for c in saved) == ["mars@3", "wave@2"]


def test_a_request_the_core_rejects_is_named_like_other_argument_errors(
    httpserver: HTTPServer, client: pyaviso.AvisoClient
) -> None:
    bodies = serve(httpserver, {"mars": 1})
    too_far = 2**64 - 1
    with pytest.raises(pyaviso.ConfigError, match="listener 'a': ") as caught:
        client.listen_many({"a": entry("mars", start_from=too_far)})
    assert caught.value.listener == "a"
    assert bodies == []


def test_failure_kinds_are_an_enum(httpserver: HTTPServer, client: pyaviso.AvisoClient) -> None:
    serve(httpserver, {"broken": None, "mars": 1})
    it = client.listen_many({"bad": entry("broken"), "ok": entry("mars")}, on_error="continue")
    it.run()
    [failure] = it.errors
    assert failure.kind is pyaviso.ListenFailureKind.LISTENER
    assert failure.kind == "listener"
