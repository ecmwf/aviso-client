"""WatchRequest builder tests."""

from __future__ import annotations

import aviso
import pytest


def test_watch_builds_live_only() -> None:
    req = aviso.WatchRequest.watch("mars")
    assert req.event_type == "mars"
    assert req.mode == "watch"


def test_watch_from_with_sequence() -> None:
    req = aviso.WatchRequest.watch_from("mars", 42)
    assert req.event_type == "mars"
    assert req.mode == "watch"


def test_watch_from_with_date_string() -> None:
    req = aviso.WatchRequest.watch_from("mars", "2026-01-01T00:00:00Z")
    assert req.event_type == "mars"


def test_replay_only_requires_resume_position() -> None:
    req = aviso.WatchRequest.replay_only("mars", 100)
    assert req.mode == "replay_only"


def test_from_rejects_bool() -> None:
    with pytest.raises(TypeError):
        aviso.WatchRequest.watch_from("mars", True)


def test_from_rejects_negative_sequence() -> None:
    with pytest.raises(ValueError):
        aviso.WatchRequest.watch_from("mars", -1)


def test_with_filter() -> None:
    req = aviso.WatchRequest.watch("mars").with_filter({"class": "od", "stream": "oper"})
    assert req.event_type == "mars"


def test_with_triggers() -> None:
    req = aviso.WatchRequest.watch("mars").with_triggers(
        [aviso.Trigger.echo(), aviso.Trigger.echo(label="x")]
    )
    assert req.event_type == "mars"


def test_listen_requires_event_or_request() -> None:
    client = aviso.AvisoClient(base_url="http://127.0.0.1:1")
    with pytest.raises(aviso.AvisoError):
        client.listen()


def test_listen_rejects_mixed_kwargs_with_request() -> None:
    client = aviso.AvisoClient(base_url="http://127.0.0.1:1")
    req = aviso.WatchRequest.watch("mars")
    with pytest.raises(aviso.AvisoError):
        client.listen(event_type="mars", request=req)


def test_listen_replay_only_requires_from() -> None:
    client = aviso.AvisoClient(base_url="http://127.0.0.1:1")
    with pytest.raises(aviso.AvisoError):
        client.listen("mars", mode="replay_only")


def test_async_client_constructs() -> None:
    client = aviso.AsyncAvisoClient(base_url="http://127.0.0.1:1")
    assert client.base_url == "http://127.0.0.1:1/"
