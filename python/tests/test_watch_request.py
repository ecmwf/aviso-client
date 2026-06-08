# (C) Copyright 2024- ECMWF and individual contributors.
#
# This software is licensed under the terms of the Apache Licence Version 2.0
# which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
# In applying this licence, ECMWF does not waive the privileges and immunities
# granted to it by virtue of its status as an intergovernmental organisation nor
# does it submit to any jurisdiction.

"""WatchRequest builder tests."""

from __future__ import annotations

import pyaviso
import pytest


def test_watch_builds_live_only() -> None:
    req = pyaviso.WatchRequest.watch("mars")
    assert req.event_type == "mars"
    assert req.mode == "watch"


def test_watch_from_with_sequence() -> None:
    req = pyaviso.WatchRequest.watch_from("mars", 42)
    assert req.event_type == "mars"
    assert req.mode == "watch"


def test_watch_from_accepts_from_underscore_kwarg() -> None:
    req = pyaviso.WatchRequest.watch_from("mars", from_=42)
    assert req.event_type == "mars"


def test_watch_from_with_date_string() -> None:
    req = pyaviso.WatchRequest.watch_from("mars", "2026-01-01T00:00:00Z")
    assert req.event_type == "mars"


def test_replay_only_requires_resume_position() -> None:
    req = pyaviso.WatchRequest.replay_only("mars", 100)
    assert req.mode == "replay_only"


def test_from_rejects_bool() -> None:
    with pytest.raises(TypeError):
        pyaviso.WatchRequest.watch_from("mars", True)


def test_from_rejects_negative_sequence() -> None:
    with pytest.raises(ValueError):
        pyaviso.WatchRequest.watch_from("mars", -1)


def test_from_rejects_int_larger_than_u64() -> None:
    with pytest.raises(ValueError) as excinfo:
        pyaviso.WatchRequest.watch_from("mars", 2**200)
    assert "u64" in str(excinfo.value) or "too large" in str(excinfo.value)


def test_with_filter() -> None:
    req = pyaviso.WatchRequest.watch("mars").with_filter({"class": "od", "stream": "oper"})
    assert req.event_type == "mars"


def test_with_triggers() -> None:
    req = pyaviso.WatchRequest.watch("mars").with_triggers(
        [pyaviso.Trigger.echo(), pyaviso.Trigger.echo(label="x")]
    )
    assert req.event_type == "mars"


def test_listen_requires_event_or_request() -> None:
    client = pyaviso.AvisoClient(base_url="http://127.0.0.1:1")
    with pytest.raises(pyaviso.AvisoError):
        client.listen()


def test_listen_rejects_mixed_kwargs_with_request() -> None:
    client = pyaviso.AvisoClient(base_url="http://127.0.0.1:1")
    req = pyaviso.WatchRequest.watch("mars")
    with pytest.raises(pyaviso.AvisoError):
        client.listen(event_type="mars", request=req)


def test_listen_replay_only_requires_from() -> None:
    client = pyaviso.AvisoClient(base_url="http://127.0.0.1:1")
    with pytest.raises(pyaviso.AvisoError):
        client.listen("mars", mode="replay_only")


def test_async_client_constructs() -> None:
    client = pyaviso.AsyncAvisoClient(base_url="http://127.0.0.1:1")
    assert client.base_url == "http://127.0.0.1:1/"
