# (C) Copyright 2024- ECMWF and individual contributors.
#
# This software is licensed under the terms of the Apache Licence Version 2.0
# which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
# In applying this licence, ECMWF does not waive the privileges and immunities
# granted to it by virtue of its status as an intergovernmental organisation nor
# does it submit to any jurisdiction.

"""A local server for the listener tests.

It answers each listener according to its event type: a replay of a few
notifications that then ends, or a refusal. Tests therefore end on their own,
and each listener's outcome is controlled.
"""

from __future__ import annotations

import json
import re
from typing import Any

from pytest_httpserver import HTTPServer
from werkzeug import Request, Response

START = "2026-09-01T00:00:00Z"


def sse(event: str, data: dict[str, Any]) -> str:
    return f"event: {event}\ndata: {json.dumps(data)}\n\n"


def replay(event_type: str, count: int) -> str:
    body = sse("replay-control", {"type": "replay_started"})
    for n in range(1, count + 1):
        body += sse(
            "replay",
            {
                "id": f"{event_type}@{n}",
                "source": "https://aviso.example",
                "type": f"int.ecmwf.aviso.{event_type}",
                "time": "2026-09-01T00:00:00Z",
                "data": {"identifier": {"n": str(n)}, "payload": None},
            },
        )
    body += sse("replay-control", {"type": "replay_completed", "topic": event_type})
    body += sse(
        "connection-closing",
        {"reason": "end_of_stream", "timestamp": "2026-09-01T01:00:00Z", "topic": event_type},
    )
    return body


def serve(httpserver: HTTPServer, replies: dict[str, int | None]) -> list[dict[str, Any]]:
    """Answers each event type with a replay of `count` notifications, or with
    a 400 when `count` is None. Returns the request bodies as they arrive."""
    bodies: list[dict[str, Any]] = []

    def handler(request: Request) -> Response:
        body = json.loads(request.data)
        bodies.append(body)
        count = replies[body["event_type"]]
        if count is None:
            return Response("unknown field 'stepp'", status=400)
        return Response(replay(body["event_type"], count), content_type="text/event-stream")

    httpserver.expect_request(
        re.compile("^/api/v1/(watch|replay)$"), method="POST"
    ).respond_with_handler(handler)
    return bodies


def entry(event_type: str, **extra: Any) -> dict[str, Any]:
    return {"event_type": event_type, "start_from": START, "mode": "replay_only", **extra}


def quiet(httpserver: HTTPServer) -> None:
    """A live listener that never delivers: the stream opens, then ends, and
    the client keeps reconnecting."""
    httpserver.expect_request(re.compile("^/api/v1/watch$"), method="POST").respond_with_data(
        sse("live-notification", {"type": "connection_established"}),
        content_type="text/event-stream",
    )
