from __future__ import annotations

import time

import pyaviso
from _helpers import receive_within

POLYGON = "0,0,1,0,1,1,0,0"
EVENT_TYPE = "test_polygon"


def test_publish_listen_roundtrips_three_notifications(
    producer_client: pyaviso.AvisoClient,
) -> None:
    expected = [1, 2, 3]
    received: list[dict] = []
    with producer_client.listen(EVENT_TYPE, filter={"polygon": POLYGON}) as iterator:
        time.sleep(0.5)
        for n in expected:
            producer_client.notify(
                event_type=EVENT_TYPE,
                identifier={"polygon": POLYGON, "date": "20260601", "time": f"{n:04d}"},
                payload={"seq": n},
            )
        for _ in expected:
            received.append(receive_within(iterator, timeout=5).payload)

    assert [r["seq"] for r in received] == expected
