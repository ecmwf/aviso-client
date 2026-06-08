# (C) Copyright 2024- ECMWF and individual contributors.
#
# This software is licensed under the terms of the Apache Licence Version 2.0
# which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
# In applying this licence, ECMWF does not waive the privileges and immunities
# granted to it by virtue of its status as an intergovernmental organisation nor
# does it submit to any jurisdiction.

from __future__ import annotations

import time

import pyaviso
from _helpers import background_publishes, receive_within

POLYGON = "10,0,11,0,11,1,10,0"
EVENT_TYPE = "test_polygon"
CONNECTION_MAX_DURATION_SEC = 15


def _publish(client: pyaviso.AvisoClient, seq: int) -> None:
    client.notify(
        event_type=EVENT_TYPE,
        identifier={"polygon": POLYGON, "date": "20260602", "time": f"{seq:04d}"},
        payload={"seq": seq},
    )


def test_listener_survives_max_duration_reached_cut(
    producer_client: pyaviso.AvisoClient,
) -> None:
    expected = {1, 2, 3, 4}
    received: list[int] = []

    def publish_sequence() -> None:
        time.sleep(0.5)
        _publish(producer_client, 1)
        time.sleep(0.5)
        _publish(producer_client, 2)
        time.sleep(CONNECTION_MAX_DURATION_SEC + 1.0)
        _publish(producer_client, 3)
        time.sleep(0.5)
        _publish(producer_client, 4)

    with producer_client.listen(EVENT_TYPE, filter={"polygon": POLYGON}) as iterator:
        time.sleep(0.5)
        with background_publishes(publish_sequence, timeout=CONNECTION_MAX_DURATION_SEC + 10):
            deadline = time.monotonic() + CONNECTION_MAX_DURATION_SEC + 10
            while set(received) < expected and time.monotonic() < deadline:
                remaining = max(deadline - time.monotonic(), 0.5)
                received.append(receive_within(iterator, timeout=remaining).payload["seq"])

    assert set(received) >= expected, f"missing items: {expected - set(received)}"
    assert len(received) <= len(expected) + 1, (
        f"at-least-once delivery should produce at most one duplicate per cut; got {received}"
    )
