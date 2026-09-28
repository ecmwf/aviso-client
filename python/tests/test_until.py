# (C) Copyright 2024- ECMWF and individual contributors.
#
# This software is licensed under the terms of the Apache Licence Version 2.0
# which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
# In applying this licence, ECMWF does not waive the privileges and immunities
# granted to it by virtue of its status as an intergovernmental organisation nor
# does it submit to any jurisdiction.

"""The replay end point, ``until=``, across the Python entry points."""

from __future__ import annotations

import ast
import inspect
import json
from collections.abc import Callable
from pathlib import Path

import pyaviso
import pytest
from pytest_httpserver import HTTPServer

_END = (
    'event: replay-control\ndata: {"type":"replay_completed","topic":"mars"}\n\n'
    'event: connection-closing\ndata: {"reason":"end_of_stream","topic":"mars"}\n\n'
)


def _replay(*sequences: int, end_sequence: int) -> str:
    """A finished replay stream with the given notifications."""
    started = {"type": "replay_started", "end_sequence": end_sequence}
    body = f"event: replay-control\ndata: {json.dumps(started)}\n\n"
    for sequence in sequences:
        data = {"id": f"mars@{sequence}", "data": {"identifier": {}, "payload": None}}
        body += f"event: replay\ndata: {json.dumps(data)}\n\n"
    return body + _END


_UNTIL_METHODS: dict[tuple[str, str], Callable[..., object]] = {
    ("AvisoClient", "listen"): pyaviso.AvisoClient.listen,
    ("AvisoClient", "listen_many"): pyaviso.AvisoClient.listen_many,
    ("AsyncAvisoClient", "listen"): pyaviso.AsyncAvisoClient.listen,
    ("AsyncAvisoClient", "listen_many"): pyaviso.AsyncAvisoClient.listen_many,
    ("WatchRequest", "replay_only"): pyaviso.WatchRequest.replay_only,
}


@pytest.mark.parametrize("filename", ["__init__.pyi", "_native.pyi"])
def test_until_in_public_stubs(filename: str) -> None:
    """Each stub declares until as the runtime does: keyword-only, int or
    str, defaulting to None, in the same position among the keywords."""
    path = Path(pyaviso.__file__).with_name(filename)
    module = ast.parse(path.read_text(encoding="utf-8"))
    found = set()
    for cls in module.body:
        if not isinstance(cls, ast.ClassDef):
            continue
        for method in cls.body:
            if not isinstance(method, ast.FunctionDef):
                continue
            key = (cls.name, method.name)
            if key not in _UNTIL_METHODS:
                continue
            keywords = [arg.arg for arg in method.args.kwonlyargs]
            assert "until" in keywords, key
            index = keywords.index("until")
            annotation = method.args.kwonlyargs[index].annotation
            assert annotation is not None, key
            assert ast.unparse(annotation) == "int | str | None", key
            default = method.args.kw_defaults[index]
            assert isinstance(default, ast.Constant) and default.value is None, key
            runtime = [
                name
                for name, parameter in inspect.signature(_UNTIL_METHODS[key]).parameters.items()
                if parameter.kind is inspect.Parameter.KEYWORD_ONLY
            ]
            assert keywords == runtime, (key, keywords, runtime)
            found.add(key)
    assert found == set(_UNTIL_METHODS)


@pytest.mark.parametrize(
    "method",
    [
        pyaviso.AvisoClient.listen,
        pyaviso.AvisoClient.listen_many,
        pyaviso.AsyncAvisoClient.listen,
        pyaviso.AsyncAvisoClient.listen_many,
        pyaviso.WatchRequest.replay_only,
    ],
)
def test_signature_exposes_until_as_a_keyword(method: Callable[..., object]) -> None:
    parameter = inspect.signature(method).parameters["until"]
    assert parameter.kind is inspect.Parameter.KEYWORD_ONLY
    assert parameter.default is None


@pytest.mark.parametrize("client_type", [pyaviso.AvisoClient, pyaviso.AsyncAvisoClient])
@pytest.mark.parametrize(
    "value,error", [(True, TypeError), (-1, ValueError), (2**200, ValueError), (1.5, TypeError)]
)
def test_until_validation_names_the_argument(
    client_type: type, value: object, error: type[Exception]
) -> None:
    client = client_type(base_url="http://127.0.0.1:1")
    with pytest.raises(error, match="until"):
        client.listen("mars", start_from=0, until=value)


@pytest.mark.parametrize("client_type", [pyaviso.AvisoClient, pyaviso.AsyncAvisoClient])
def test_until_needs_a_replay(client_type: type) -> None:
    client = client_type(base_url="http://127.0.0.1:1")
    with pytest.raises(pyaviso.AvisoError, match=r"until= ends a replay.*mode='replay_only'"):
        client.listen("mars", start_from=0, until=5, mode="watch")
    with pytest.raises(
        pyaviso.AvisoError, match=r"until= ends a replay, which also needs start_from"
    ):
        client.listen("mars", until=5)
    with pytest.raises(pyaviso.AvisoError, match="requires start_from"):
        client.listen("mars", until=5, mode="replay_only")
    with pytest.raises(pyaviso.AvisoError, match=r"mutually exclusive.*until"):
        client.listen(request=pyaviso.WatchRequest.watch("mars"), until=5)


@pytest.mark.parametrize("client_type", [pyaviso.AvisoClient, pyaviso.AsyncAvisoClient])
@pytest.mark.parametrize("until", [10, 3])
def test_a_sequence_end_not_after_the_start_is_refused(client_type: type, until: int) -> None:
    client = client_type(base_url="http://127.0.0.1:1")
    with pytest.raises(pyaviso.ConfigError, match=f"end sequence {until} is not after its start"):
        client.listen("mars", start_from=10, until=until)


def test_replay_only_builder_takes_until_by_keyword_only() -> None:
    request = pyaviso.WatchRequest.replay_only("mars", 0, until=5)
    assert request.mode == "replay_only"
    signature = inspect.signature(pyaviso.WatchRequest.replay_only)
    with pytest.raises(TypeError):
        signature.bind("mars", 0, 5)


_WIRE_CASES = [
    (0, 2, {"from_id": "1", "to_id": "2"}),
    (
        "2026-06-01T00:00:00Z",
        "2026-06-02T00:00:00Z",
        {"from_date": "2026-06-01T00:00:00Z", "to_date": "2026-06-02T00:00:00Z"},
    ),
    (0, "2026-06-02T00:00:00Z", {"from_id": "1", "to_date": "2026-06-02T00:00:00Z"}),
]


async def _read_all(stream: object) -> list[int]:
    sequences: list[int] = []
    if isinstance(stream, pyaviso.AsyncNotificationIterator):
        async with stream:
            sequences = [n.sequence async for n in stream]
    else:
        assert isinstance(stream, pyaviso.NotificationIterator)
        with stream:
            sequences = [n.sequence for n in stream]
    return sequences


def _expect_replay(httpserver: HTTPServer, wire: dict[str, str]) -> None:
    httpserver.expect_request(
        "/api/v1/replay",
        method="POST",
        json={"event_type": "mars", "identifier": {}, **wire},
    ).respond_with_data(_replay(1, 2, end_sequence=2), content_type="text/event-stream")


@pytest.mark.parametrize("mode", [None, "replay_only", pyaviso.WatchMode.REPLAY_ONLY])
@pytest.mark.parametrize("start_from,until,wire", _WIRE_CASES)
async def test_listen_until_wire(
    httpserver: HTTPServer,
    mode: pyaviso.WatchMode | str | None,
    start_from: int | str,
    until: int | str,
    wire: dict[str, str],
) -> None:
    _expect_replay(httpserver, wire)
    for client_type in (pyaviso.AvisoClient, pyaviso.AsyncAvisoClient):
        client = client_type(base_url=httpserver.url_for("/"))
        stream = client.listen("mars", start_from=start_from, until=until, mode=mode)
        assert await _read_all(stream) == [1, 2]
    httpserver.check_assertions()


@pytest.mark.parametrize("start_from,until,wire", _WIRE_CASES)
async def test_watch_request_until_wire(
    httpserver: HTTPServer, start_from: int | str, until: int | str, wire: dict[str, str]
) -> None:
    _expect_replay(httpserver, wire)
    request = pyaviso.WatchRequest.replay_only("mars", start_from, until=until)
    for client_type in (pyaviso.AvisoClient, pyaviso.AsyncAvisoClient):
        client = client_type(base_url=httpserver.url_for("/"))
        assert await _read_all(client.listen(request=request)) == [1, 2]
    httpserver.check_assertions()


async def test_listen_many_until_is_shared_or_per_listener(httpserver: HTTPServer) -> None:
    for event, to_id in (("mars", "2"), ("alerts", "5")):
        httpserver.expect_request(
            "/api/v1/replay",
            method="POST",
            json={"event_type": event, "identifier": {}, "from_id": "1", "to_id": to_id},
        ).respond_with_data(
            _replay(1, end_sequence=int(to_id)).replace("mars@", f"{event}@"),
            content_type="text/event-stream",
        )
    listeners = {
        "shared": {"event_type": "mars"},
        "own": {"event_type": "alerts", "until": 5},
    }
    client = pyaviso.AvisoClient(base_url=httpserver.url_for("/"))
    with client.listen_many(listeners, start_from=0, until=2) as notifications:
        names = sorted(name for name, _ in notifications)
    assert names == ["own", "shared"]
    httpserver.check_assertions()


def test_listen_many_checks_the_shared_until_and_entry_keys() -> None:
    client = pyaviso.AvisoClient(base_url="http://127.0.0.1:1")
    with pytest.raises(ValueError, match="until sequence must be non-negative"):
        client.listen_many({"a": {"event_type": "mars"}}, start_from=0, until=-1)
    with pytest.raises(pyaviso.AvisoError, match=r"listener 'a'.*until= ends a replay"):
        client.listen_many({"a": {"event_type": "mars", "mode": "watch", "until": 3}})
