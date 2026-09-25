# (C) Copyright 2024- ECMWF and individual contributors.
#
# This software is licensed under the terms of the Apache Licence Version 2.0
# which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
# In applying this licence, ECMWF does not waive the privileges and immunities
# granted to it by virtue of its status as an intergovernmental organisation nor
# does it submit to any jurisdiction.

"""Resume keywords reach the wire consistently across Python entry points."""

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


@pytest.mark.parametrize("filename", ["__init__.pyi", "_native.pyi"])
def test_start_from_in_public_stubs(filename: str) -> None:
    path = Path(pyaviso.__file__).with_name(filename)
    module = ast.parse(path.read_text(encoding="utf-8"))
    expected = {
        ("AvisoClient", "listen"),
        ("AsyncAvisoClient", "listen"),
        ("WatchRequest", "watch_from"),
        ("WatchRequest", "replay_only"),
    }
    found = set()
    for cls in module.body:
        if not isinstance(cls, ast.ClassDef):
            continue
        for method in cls.body:
            if isinstance(method, ast.FunctionDef) and (cls.name, method.name) in expected:
                arguments = method.args.args + method.args.kwonlyargs
                assert "start_from" in {arg.arg for arg in arguments}
                found.add((cls.name, method.name))
    assert found == expected


@pytest.mark.parametrize("client_type", [pyaviso.AvisoClient, pyaviso.AsyncAvisoClient])
def test_start_from_conflicts_with_request(client_type: type) -> None:
    client = client_type(base_url="http://127.0.0.1:1")
    with pytest.raises(pyaviso.AvisoError, match=r"mutually exclusive.*start_from"):
        client.listen(request=pyaviso.WatchRequest.watch("mars"), start_from=0)


@pytest.mark.parametrize("builder", ["watch_from", "replay_only"])
@pytest.mark.parametrize("start_from", [0, 42, "2026-06-01T00:00:00Z"])
def test_builder_start_from_keyword(builder: str, start_from: int | str) -> None:
    method = getattr(pyaviso.WatchRequest, builder)
    keyword = method("mars", start_from=start_from)
    positional = method("mars", start_from)
    assert keyword.event_type == positional.event_type == "mars"
    assert keyword.mode == positional.mode == ("watch" if builder == "watch_from" else builder)


@pytest.mark.parametrize(
    "method",
    [
        pyaviso.AvisoClient.listen,
        pyaviso.AsyncAvisoClient.listen,
        pyaviso.WatchRequest.watch_from,
        pyaviso.WatchRequest.replay_only,
    ],
)
def test_signature_exposes_start_from(method: Callable[..., object]) -> None:
    signature = inspect.signature(method)
    assert "start_from" in signature.parameters
    assert "from_" not in signature.parameters


@pytest.mark.parametrize("client_type", [pyaviso.AvisoClient, pyaviso.AsyncAvisoClient])
def test_listen_rejects_old_keyword(client_type: type) -> None:
    client = client_type(base_url="http://127.0.0.1:1")
    with pytest.raises(TypeError, match="unexpected keyword argument 'from_'"):
        client.listen("mars", **{"from_": 0})


@pytest.mark.parametrize("builder", ["watch_from", "replay_only"])
def test_builder_rejects_old_keyword(builder: str) -> None:
    with pytest.raises(TypeError, match="unexpected keyword argument 'from_'"):
        getattr(pyaviso.WatchRequest, builder)("mars", **{"from_": 0})


@pytest.mark.parametrize("client_type", [pyaviso.AvisoClient, pyaviso.AsyncAvisoClient])
@pytest.mark.parametrize(
    "value,error", [(True, TypeError), (-1, ValueError), (2**200, ValueError), (1.5, TypeError)]
)
def test_listen_start_from_validation(
    client_type: type, value: object, error: type[Exception]
) -> None:
    client = client_type(base_url="http://127.0.0.1:1")
    with pytest.raises(error, match="start_from"):
        client.listen("mars", start_from=value)


@pytest.mark.parametrize("mode", ["watch", "replay_only"])
@pytest.mark.parametrize("builder", [False, True])
@pytest.mark.parametrize(
    "start_from,wire",
    [
        (0, {"from_id": "1"}),
        (42, {"from_id": "43"}),
        ("2026-06-01T00:00:00Z", {"from_date": "2026-06-01T00:00:00Z"}),
    ],
)
async def test_start_from_wire(
    httpserver: HTTPServer, mode: str, builder: bool, start_from: int | str, wire: dict[str, str]
) -> None:
    sequence = start_from + 1 if isinstance(start_from, int) else 43
    data = {"id": f"mars@{sequence}", "data": {"identifier": {}, "payload": None}}
    event = (
        'event: replay-control\ndata: {"type":"replay_started"}\n\n'
        f"event: replay\ndata: {json.dumps(data)}\n\n"
    )
    httpserver.expect_request(
        f"/api/v1/{'watch' if mode == 'watch' else 'replay'}",
        method="POST",
        json={"event_type": "mars", "identifier": {}, **wire},
    ).respond_with_data(event + _END, content_type="text/event-stream")
    if mode == "watch":
        # The finite response can reconnect before the consumer closes it.
        # Resume must use the delivered sequence, including for a date start.
        httpserver.expect_request(
            "/api/v1/watch",
            method="POST",
            json={"event_type": "mars", "identifier": {}, "from_id": str(sequence + 1)},
        ).respond_with_data("busy", status=503, headers={"Retry-After": "60"})
    for client_type in (pyaviso.AvisoClient, pyaviso.AsyncAvisoClient):
        client = client_type(base_url=httpserver.url_for("/"))
        if builder:
            method = getattr(pyaviso.WatchRequest, "watch_from" if mode == "watch" else mode)
            stream = client.listen(request=method("mars", start_from=start_from))
        else:
            stream = client.listen("mars", start_from=start_from, mode=mode)
        if isinstance(
            stream, (pyaviso.AsyncNotificationIterator, pyaviso.AsyncFunctionTriggerIterator)
        ):
            async with stream:
                assert (await anext(stream)).sequence == sequence
                if mode == "replay_only":
                    with pytest.raises(StopAsyncIteration):
                        await anext(stream)
        else:
            with stream:
                assert next(stream).sequence == sequence
                if mode == "replay_only":
                    with pytest.raises(StopIteration):
                        next(stream)
    httpserver.check_assertions()
