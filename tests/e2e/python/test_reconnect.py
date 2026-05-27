from __future__ import annotations

import threading
import time

import aviso

POLYGON = "10,0,11,0,11,1,10,0"
EVENT_TYPE = "test_polygon"
CONNECTION_MAX_DURATION_SEC = 15


def _publish(client: aviso.AvisoClient, seq: int) -> None:
    client.notify(
        event_type=EVENT_TYPE,
        identifier={"polygon": POLYGON, "date": "20260602", "time": f"{seq:04d}"},
        payload={"seq": seq},
    )


def _delayed_publish(client: aviso.AvisoClient, seq: int, delay_sec: float) -> None:
    def run() -> None:
        time.sleep(delay_sec)
        _publish(client, seq)

    threading.Thread(target=run, daemon=True).start()


def test_listener_survives_max_duration_reached_cut(
    producer_client: aviso.AvisoClient,
) -> None:
    expected = {1, 2, 3, 4}
    received: list[int] = []
    with producer_client.listen(EVENT_TYPE, filter={"polygon": POLYGON}) as iterator:
        time.sleep(0.5)
        _delayed_publish(producer_client, 1, 0.5)
        _delayed_publish(producer_client, 2, 1.0)
        _delayed_publish(producer_client, 3, CONNECTION_MAX_DURATION_SEC + 2.0)
        _delayed_publish(producer_client, 4, CONNECTION_MAX_DURATION_SEC + 3.0)
        deadline = time.monotonic() + CONNECTION_MAX_DURATION_SEC + 10
        while set(received) < expected and time.monotonic() < deadline:
            notification = next(iterator)
            received.append(notification.payload["seq"])

    assert set(received) >= expected, f"missing items: {expected - set(received)}"
    assert len(received) <= len(expected) + 1, (
        f"at-most-one duplicate per cut per D2; got {received}"
    )
