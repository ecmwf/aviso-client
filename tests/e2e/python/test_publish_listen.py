from __future__ import annotations

import threading
import time

import aviso

POLYGON = "0,0,1,0,1,1,0,0"
EVENT_TYPE = "test_polygon"


def _publish_in_thread(client: aviso.AvisoClient, sequences: list[int]) -> None:
    def run() -> None:
        for n in sequences:
            client.notify(
                event_type=EVENT_TYPE,
                identifier={"polygon": POLYGON, "date": "20260601", "time": f"{n:04d}"},
                payload={"seq": n},
            )
            time.sleep(0.05)

    threading.Thread(target=run, daemon=True).start()


def test_publish_listen_roundtrips_three_notifications(
    producer_client: aviso.AvisoClient,
) -> None:
    expected = [1, 2, 3]
    received: list[dict] = []
    with producer_client.listen(EVENT_TYPE, filter={"polygon": POLYGON}) as iterator:
        time.sleep(0.5)
        _publish_in_thread(producer_client, expected)
        for _ in range(len(expected)):
            notification = next(iterator)
            received.append(notification.payload)

    assert [r["seq"] for r in received] == expected
