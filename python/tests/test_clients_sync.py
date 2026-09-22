# (C) Copyright 2024- ECMWF and individual contributors.
#
# This software is licensed under the terms of the Apache Licence Version 2.0
# which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
# In applying this licence, ECMWF does not waive the privileges and immunities
# granted to it by virtue of its status as an intergovernmental organisation nor
# does it submit to any jurisdiction.

"""AvisoClient (sync) construction and method-shape tests.

Network-driven tests against a mock server live in separate files (one
per method) so they stay readable. These tests cover the construction
surface, the keyword-only argument rules, the context-manager protocol,
and that calling against an unreachable base_url raises TransportError
or HttpError per the documented contract.
"""

from __future__ import annotations

from typing import Any

import pyaviso
import pytest


def test_construct_with_only_base_url() -> None:
    client = pyaviso.AvisoClient(base_url="http://127.0.0.1:1")
    assert client.base_url == "http://127.0.0.1:1/"


def test_construct_with_auth() -> None:
    client = pyaviso.AvisoClient(
        base_url="http://127.0.0.1:1",
        auth=pyaviso.Bearer("opaque-jwt-here"),
    )
    assert client.base_url == "http://127.0.0.1:1/"


def test_empty_token_raises_config_error_on_provider() -> None:
    with pytest.raises(pyaviso.ConfigError):
        pyaviso.Bearer("")


def test_invalid_base_url_raises_config_error() -> None:
    with pytest.raises(pyaviso.ConfigError):
        pyaviso.AvisoClient(base_url="not a url")


def test_repr_shows_base_url() -> None:
    client = pyaviso.AvisoClient(base_url="http://127.0.0.1:1")
    rendered = repr(client)
    assert "AvisoClient" in rendered
    assert "127.0.0.1" in rendered


def test_repr_does_not_show_credentials_embedded_in_the_url() -> None:
    client = pyaviso.AvisoClient(base_url="https://operator:hunter2@aviso.example.org/")
    rendered = repr(client)
    assert "aviso.example.org" in rendered
    assert "hunter2" not in rendered
    assert "operator" not in rendered


def test_context_manager_returns_self() -> None:
    with pyaviso.AvisoClient(base_url="http://127.0.0.1:1") as client:
        assert client.base_url == "http://127.0.0.1:1/"


def test_notify_against_unreachable_raises_transport_error() -> None:
    client = pyaviso.AvisoClient(base_url="http://127.0.0.1:1")
    with pytest.raises(pyaviso.TransportError):
        client.notify(event_type="mars")


def test_schema_against_unreachable_raises_transport_error() -> None:
    client = pyaviso.AvisoClient(base_url="http://127.0.0.1:1")
    with pytest.raises(pyaviso.TransportError):
        client.schema()


def test_listen_accepts_triggers_kwarg() -> None:
    client = pyaviso.AvisoClient(base_url="http://127.0.0.1:1")
    iterator = client.listen(
        "test_polygon",
        filter={"polygon": "0,0,1,0,1,1,0,0"},
        triggers=[pyaviso.Trigger.echo()],
    )
    iterator.close()


def test_listen_accepts_triggers_as_tuple() -> None:
    client = pyaviso.AvisoClient(base_url="http://127.0.0.1:1")
    iterator = client.listen(
        "test_polygon",
        filter={"polygon": "0,0,1,0,1,1,0,0"},
        triggers=(pyaviso.Trigger.echo(), pyaviso.Trigger.echo()),
    )
    iterator.close()


def test_listen_rejects_bare_trigger_value() -> None:
    client = pyaviso.AvisoClient(base_url="http://127.0.0.1:1")
    bare: Any = pyaviso.Trigger.echo()
    with pytest.raises(pyaviso.AvisoError):
        client.listen(
            "test_polygon",
            filter={"polygon": "0,0,1,0,1,1,0,0"},
            triggers=bare,
        )


def test_listen_rejects_triggers_string() -> None:
    client = pyaviso.AvisoClient(base_url="http://127.0.0.1:1")
    bogus: Any = "echo"
    with pytest.raises(pyaviso.AvisoError):
        client.listen(
            "test_polygon",
            filter={"polygon": "0,0,1,0,1,1,0,0"},
            triggers=bogus,
        )


def test_listen_triggers_with_request_raises() -> None:
    client = pyaviso.AvisoClient(base_url="http://127.0.0.1:1")
    request = pyaviso.WatchRequest.watch("test_polygon").with_filter({"polygon": "0,0,1,0,1,1,0,0"})
    with pytest.raises(pyaviso.AvisoError):
        client.listen(request=request, triggers=[pyaviso.Trigger.echo()])


def test_listen_empty_triggers_with_request_still_raises() -> None:
    client = pyaviso.AvisoClient(base_url="http://127.0.0.1:1")
    request = pyaviso.WatchRequest.watch("test_polygon").with_filter({"polygon": "0,0,1,0,1,1,0,0"})
    with pytest.raises(pyaviso.AvisoError):
        client.listen(request=request, triggers=[])


def test_async_listen_accepts_triggers_kwarg() -> None:
    import asyncio

    client = pyaviso.AsyncAvisoClient(base_url="http://127.0.0.1:1")

    async def drive() -> None:
        iterator = client.listen(
            "test_polygon",
            filter={"polygon": "0,0,1,0,1,1,0,0"},
            triggers=[pyaviso.Trigger.echo()],
        )
        await iterator.aclose()

    asyncio.run(drive())


def test_async_listen_triggers_with_request_raises() -> None:
    client = pyaviso.AsyncAvisoClient(base_url="http://127.0.0.1:1")
    request = pyaviso.WatchRequest.watch("test_polygon").with_filter({"polygon": "0,0,1,0,1,1,0,0"})
    with pytest.raises(pyaviso.AvisoError):
        client.listen(request=request, triggers=[pyaviso.Trigger.echo()])
