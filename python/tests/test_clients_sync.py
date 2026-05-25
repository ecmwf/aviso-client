"""AvisoClient (sync) construction and method-shape tests.

Network-driven tests against a mock server live in separate files (one
per method) so they stay readable. These tests cover the construction
surface, the keyword-only argument rules, the context-manager protocol,
and that calling against an unreachable base_url raises TransportError
or HttpError per the documented contract.
"""

from __future__ import annotations

from typing import Any

import aviso
import pytest


def test_construct_with_only_base_url() -> None:
    client = aviso.AvisoClient(base_url="http://127.0.0.1:1")
    assert client.base_url == "http://127.0.0.1:1/"


def test_construct_with_auth() -> None:
    client = aviso.AvisoClient(
        base_url="http://127.0.0.1:1",
        auth=aviso.Bearer("opaque-jwt-here"),
    )
    assert client.base_url == "http://127.0.0.1:1/"


def test_empty_token_raises_config_error_on_provider() -> None:
    with pytest.raises(aviso.ConfigError):
        aviso.Bearer("")


def test_invalid_base_url_raises_config_error() -> None:
    with pytest.raises(aviso.ConfigError):
        aviso.AvisoClient(base_url="not a url")


def test_repr_shows_base_url() -> None:
    client = aviso.AvisoClient(base_url="http://127.0.0.1:1")
    rendered = repr(client)
    assert "AvisoClient" in rendered
    assert "127.0.0.1" in rendered


def test_context_manager_returns_self() -> None:
    with aviso.AvisoClient(base_url="http://127.0.0.1:1") as client:
        assert client.base_url == "http://127.0.0.1:1/"


def test_notify_against_unreachable_raises_transport_error() -> None:
    client = aviso.AvisoClient(base_url="http://127.0.0.1:1")
    with pytest.raises(aviso.TransportError):
        client.notify(event_type="mars")


def test_schema_against_unreachable_raises_transport_error() -> None:
    client = aviso.AvisoClient(base_url="http://127.0.0.1:1")
    with pytest.raises(aviso.TransportError):
        client.schema()


def test_listen_accepts_triggers_kwarg() -> None:
    client = aviso.AvisoClient(base_url="http://127.0.0.1:1")
    iterator = client.listen(
        "test_polygon",
        filter={"polygon": "0,0,1,0,1,1,0,0"},
        triggers=[aviso.Trigger.echo()],
    )
    iterator.close()


def test_listen_accepts_triggers_as_tuple() -> None:
    client = aviso.AvisoClient(base_url="http://127.0.0.1:1")
    iterator = client.listen(
        "test_polygon",
        filter={"polygon": "0,0,1,0,1,1,0,0"},
        triggers=(aviso.Trigger.echo(), aviso.Trigger.echo()),
    )
    iterator.close()


def test_listen_rejects_bare_trigger_value() -> None:
    client = aviso.AvisoClient(base_url="http://127.0.0.1:1")
    bare: Any = aviso.Trigger.echo()
    with pytest.raises(aviso.AvisoError):
        client.listen(
            "test_polygon",
            filter={"polygon": "0,0,1,0,1,1,0,0"},
            triggers=bare,
        )


def test_listen_rejects_triggers_string() -> None:
    client = aviso.AvisoClient(base_url="http://127.0.0.1:1")
    bogus: Any = "echo"
    with pytest.raises(aviso.AvisoError):
        client.listen(
            "test_polygon",
            filter={"polygon": "0,0,1,0,1,1,0,0"},
            triggers=bogus,
        )


def test_listen_triggers_with_request_raises() -> None:
    client = aviso.AvisoClient(base_url="http://127.0.0.1:1")
    request = aviso.WatchRequest.watch("test_polygon").with_filter({"polygon": "0,0,1,0,1,1,0,0"})
    with pytest.raises(aviso.AvisoError):
        client.listen(request=request, triggers=[aviso.Trigger.echo()])


def test_listen_empty_triggers_with_request_still_raises() -> None:
    client = aviso.AvisoClient(base_url="http://127.0.0.1:1")
    request = aviso.WatchRequest.watch("test_polygon").with_filter({"polygon": "0,0,1,0,1,1,0,0"})
    with pytest.raises(aviso.AvisoError):
        client.listen(request=request, triggers=[])
