# (C) Copyright 2024- ECMWF and individual contributors.
#
# This software is licensed under the terms of the Apache Licence Version 2.0
# which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
# In applying this licence, ECMWF does not waive the privileges and immunities
# granted to it by virtue of its status as an intergovernmental organisation nor
# does it submit to any jurisdiction.

"""Tests for publishing notifications with JSON-valued identifiers."""

from __future__ import annotations

import asyncio
import json
from typing import Any

import pyaviso
import pytest
from werkzeug.wrappers import Request, Response


def _success(request: Request, identifiers: list[dict[str, Any]]) -> Response:
    identifiers.append(request.get_json()["identifier"])
    body = json.dumps(
        {
            "status": "success",
            "request_id": "req-spatial",
            "processed_at": "2026-05-17T12:34:56Z",
        }
    )
    return Response(body, status=200, content_type="application/json")


def test_sync_notify_sends_structured_identifier(httpserver: Any) -> None:
    identifiers: list[dict[str, Any]] = []
    httpserver.expect_request("/api/v1/notification", method="POST").respond_with_handler(
        lambda request: _success(request, identifiers)
    )
    client = pyaviso.AvisoClient(base_url=httpserver.url_for("/"))

    client.notify(
        event_type="observations",
        identifier={"point": [46.0, 8.0], "polygon": [[46.0, 8.0], [47.0, 9.0]]},
    )

    assert identifiers == [{"point": [46.0, 8.0], "polygon": [[46.0, 8.0], [47.0, 9.0]]}]


def test_async_notify_sends_structured_identifier(httpserver: Any) -> None:
    identifiers: list[dict[str, Any]] = []
    httpserver.expect_request("/api/v1/notification", method="POST").respond_with_handler(
        lambda request: _success(request, identifiers)
    )
    client = pyaviso.AsyncAvisoClient(base_url=httpserver.url_for("/"))

    async def publish() -> None:
        await client.notify(
            event_type="observations",
            identifier={"point_cloud": [[46.0, 8.0], [47.0, 9.0]]},
        )

    asyncio.run(publish())

    assert identifiers == [{"point_cloud": [[46.0, 8.0], [47.0, 9.0]]}]


@pytest.mark.parametrize("value", [float("nan"), float("inf"), float("-inf")])
def test_sync_notify_rejects_nested_non_finite_identifier(value: float) -> None:
    client = pyaviso.AvisoClient(base_url="http://127.0.0.1:1")
    with pytest.raises(TypeError, match="NaN or infinity"):
        client.notify(event_type="observations", identifier={"point_cloud": [[46.0, value]]})


@pytest.mark.parametrize("value", [float("nan"), float("inf"), float("-inf")])
def test_async_notify_rejects_nested_non_finite_identifier(value: float) -> None:
    client = pyaviso.AsyncAvisoClient(base_url="http://127.0.0.1:1")

    async def publish() -> None:
        await client.notify(
            event_type="observations",
            identifier={"point_cloud": [[46.0, value]]},
        )

    with pytest.raises(TypeError, match="NaN or infinity"):
        asyncio.run(publish())


def test_sync_notify_rejects_identifier_cycle_before_network_io() -> None:
    client = pyaviso.AvisoClient(base_url="http://127.0.0.1:1")
    cycle: list[Any] = []
    cycle.append(cycle)

    with pytest.raises(TypeError, match="cyclic containers"):
        client.notify(event_type="observations", identifier={"value": cycle})


def test_sync_notify_rejects_deep_identifier_before_network_io() -> None:
    client = pyaviso.AvisoClient(base_url="http://127.0.0.1:1")
    nested: Any = "leaf"
    for _ in range(101):
        nested = [nested]

    with pytest.raises(ValueError, match="100 nested containers"):
        client.notify(event_type="observations", identifier={"value": nested})


def test_async_notify_rejects_identifier_cycle_before_creating_awaitable() -> None:
    client = pyaviso.AsyncAvisoClient(base_url="http://127.0.0.1:1")
    cycle: dict[str, Any] = {}
    cycle["self"] = cycle

    with pytest.raises(TypeError, match="cyclic containers"):
        client.notify(event_type="observations", identifier=cycle)
