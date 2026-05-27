from __future__ import annotations

import threading
import time
from pathlib import Path

import aviso

POLYGON = "20,0,21,0,21,1,20,0"
EVENT_TYPE = "test_polygon"


def _publish(client: aviso.AvisoClient, seq: int) -> None:
    client.notify(
        event_type=EVENT_TYPE,
        identifier={"polygon": POLYGON, "date": "20260604", "time": f"{seq:04d}"},
        payload={"seq": seq},
    )


def _delayed_publish(client: aviso.AvisoClient, seq: int, delay_sec: float) -> None:
    def run() -> None:
        time.sleep(delay_sec)
        _publish(client, seq)

    threading.Thread(target=run, daemon=True).start()


def test_resume_picks_up_after_simulated_restart(
    base_url: str,
    producer_auth: aviso.Basic,
    tmp_path: Path,
) -> None:
    state_path = tmp_path / "state.json"

    first_received: list[int] = []
    with (
        aviso.AvisoClient(
            base_url=base_url,
            auth=producer_auth,
            state_store=aviso.JsonFileStore(str(state_path)),
        ) as first,
        first.listen(EVENT_TYPE, filter={"polygon": POLYGON}) as iterator,
    ):
        time.sleep(0.5)
        _delayed_publish(first, 1, 0.5)
        _delayed_publish(first, 2, 1.0)
        for _ in range(2):
            first_received.append(next(iterator).payload["seq"])

    assert first_received == [1, 2]
    assert state_path.exists(), "JsonFileStore must persist state on clean shutdown"

    second_received: list[int] = []
    with (
        aviso.AvisoClient(
            base_url=base_url,
            auth=producer_auth,
            state_store=aviso.JsonFileStore(str(state_path)),
        ) as second,
        second.listen(EVENT_TYPE, filter={"polygon": POLYGON}) as iterator,
    ):
        time.sleep(0.5)
        _delayed_publish(second, 3, 0.5)
        deadline = time.monotonic() + 10.0
        while 3 not in second_received and time.monotonic() < deadline:
            second_received.append(next(iterator).payload["seq"])

    assert 3 in second_received, f"item 3 must arrive after resume; got {second_received}"
    assert 1 not in second_received, (
        f"item 1 was committed; should never replay; got {second_received}"
    )
    assert len(second_received) <= 2, (
        f"at most one replay per at-least-once contract; got {second_received}"
    )
