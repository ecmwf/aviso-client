from __future__ import annotations

import time

import pyaviso
from _helpers import receive_within

POLYGON = "0,20,1,20,1,21,0,20"
EVENT_TYPE = "test_polygon"


def test_notify_many_roundtrips_a_batch(producer_client: pyaviso.AvisoClient) -> None:
    expected = [1, 2, 3]
    notifications = [
        {
            "event_type": EVENT_TYPE,
            "identifier": {"polygon": POLYGON, "date": "20260611", "time": f"{n:04d}"},
            "payload": {"seq": n},
        }
        for n in expected
    ]

    received: list[int] = []
    with producer_client.listen(EVENT_TYPE, filter={"polygon": POLYGON}) as iterator:
        time.sleep(0.5)
        results = producer_client.notify_many(notifications)
        assert [r.ok for r in results] == [True, True, True]
        for _ in expected:
            received.append(receive_within(iterator, timeout=5).payload["seq"])

    assert sorted(received) == expected
