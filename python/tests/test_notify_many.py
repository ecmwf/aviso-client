"""notify_many tests for the sync and async clients.

notify_many returns one NotifyResult per input notification, in input
order, and never raises on a per-item failure: a failed item carries the
exception in ``error`` while successful items carry a ``response``. A
malformed input list is a call-level error and sends nothing.
"""

from __future__ import annotations

import asyncio
import json
from typing import Any

import pyaviso
import pytest
from werkzeug.wrappers import Request, Response


def _notification_handler(request: Request) -> Response:
    body = request.get_json()
    event_type = body["event_type"]
    if event_type == "bad":
        return Response("rejected", status=400)
    payload = json.dumps(
        {"status": "success", "request_id": event_type, "processed_at": "2026-05-17T12:34:56Z"}
    )
    return Response(payload, status=200, content_type="application/json")


def test_empty_returns_empty_list() -> None:
    client = pyaviso.AvisoClient(base_url="http://127.0.0.1:1")
    assert client.notify_many([]) == []


def test_unreachable_returns_per_item_transport_errors() -> None:
    client = pyaviso.AvisoClient(base_url="http://127.0.0.1:1")
    results = client.notify_many([{"event_type": "mars"}, {"event_type": "mars"}])
    assert len(results) == 2
    for index, result in enumerate(results):
        assert result.index == index
        assert not result.ok
        assert result.response is None
        assert isinstance(result.error, pyaviso.TransportError)


def test_rejects_non_dict_element_without_sending() -> None:
    client = pyaviso.AvisoClient(base_url="http://127.0.0.1:1")
    bad: Any = ["mars"]
    with pytest.raises(TypeError):
        client.notify_many([bad])


def test_rejects_missing_event_type_without_sending() -> None:
    client = pyaviso.AvisoClient(base_url="http://127.0.0.1:1")
    with pytest.raises(ValueError):
        client.notify_many([{"identifier": {"class": "od"}}])


def test_preserves_input_order(httpserver: Any) -> None:
    httpserver.expect_request("/api/v1/notification", method="POST").respond_with_handler(
        _notification_handler
    )
    client = pyaviso.AvisoClient(base_url=httpserver.url_for("/"))
    results = client.notify_many([{"event_type": f"e{i}"} for i in range(3)])
    assert all(r.ok for r in results)
    request_ids = []
    for result in results:
        assert result.response is not None
        request_ids.append(result.response.request_id)
    assert request_ids == ["e0", "e1", "e2"]


def test_reports_per_item_errors(httpserver: Any) -> None:
    httpserver.expect_request("/api/v1/notification", method="POST").respond_with_handler(
        _notification_handler
    )
    client = pyaviso.AvisoClient(base_url=httpserver.url_for("/"))
    results = client.notify_many([{"event_type": "ok"}, {"event_type": "bad"}])
    assert results[0].ok
    assert results[0].response is not None
    assert results[0].response.request_id == "ok"
    assert not results[1].ok
    assert isinstance(results[1].error, pyaviso.HttpError)
    assert results[1].error.status == 400


def test_async_notify_many(httpserver: Any) -> None:
    httpserver.expect_request("/api/v1/notification", method="POST").respond_with_handler(
        _notification_handler
    )
    client = pyaviso.AsyncAvisoClient(base_url=httpserver.url_for("/"))

    async def drive() -> list[Any]:
        return await client.notify_many([{"event_type": "ok"}, {"event_type": "bad"}])

    results = asyncio.run(drive())
    assert results[0].ok
    assert not results[1].ok
    assert isinstance(results[1].error, pyaviso.HttpError)
