# (C) Copyright 2024- ECMWF and individual contributors.
#
# This software is licensed under the terms of the Apache Licence Version 2.0
# which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
# In applying this licence, ECMWF does not waive the privileges and immunities
# granted to it by virtue of its status as an intergovernmental organisation nor
# does it submit to any jurisdiction.

"""Stream decoding tests for JSON-valued notification identifiers."""

from __future__ import annotations

import json
from typing import Any

import pyaviso


def test_stream_preserves_point_cloud_identifier(httpserver: Any) -> None:
    event = {
        "id": "observations@7",
        "data": {
            "identifier": {"point_cloud": [[46.0, 8.0], [47.0, 9.0]]},
            "payload": None,
        },
    }
    body = f"event: live-notification\ndata: {json.dumps(event)}\n\n"
    httpserver.expect_request("/api/v1/watch", method="POST").respond_with_data(
        body,
        content_type="text/event-stream",
    )
    client = pyaviso.AvisoClient(base_url=httpserver.url_for("/"))
    iterator = client.listen("observations", filter={})

    notification = next(iterator)
    iterator.close()

    assert notification.identifier == {"point_cloud": [[46.0, 8.0], [47.0, 9.0]]}
