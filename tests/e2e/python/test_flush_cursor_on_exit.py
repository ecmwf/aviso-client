from __future__ import annotations

import threading
import time
from pathlib import Path

import aviso

POLYGON = "70,0,71,0,71,1,70,0"
EVENT_TYPE = "test_polygon"


def _publish(client: aviso.AvisoClient, seq: int) -> None:
    client.notify(
        event_type=EVENT_TYPE,
        identifier={"polygon": POLYGON, "date": "20260608", "time": f"{seq:04d}"},
        payload={"seq": seq},
    )


def _delayed_publish(client: aviso.AvisoClient, seq: int, delay_sec: float) -> None:
    def run() -> None:
        time.sleep(delay_sec)
        _publish(client, seq)

    threading.Thread(target=run, daemon=True).start()


def test_flush_cursor_on_exit_prevents_replay_after_clean_shutdown(
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
            flush_cursor_on_exit=True,
        ) as first,
        first.listen(EVENT_TYPE, filter={"polygon": POLYGON}) as iterator,
    ):
        time.sleep(0.5)
        _delayed_publish(first, 1, 0.5)
        _delayed_publish(first, 2, 1.0)
        for _ in range(2):
            first_received.append(next(iterator).payload["seq"])

    assert first_received == [1, 2]
    assert state_path.exists(), "JsonFileStore must persist on clean shutdown"

    second_received: list[int] = []
    with (
        aviso.AvisoClient(
            base_url=base_url,
            auth=producer_auth,
            state_store=aviso.JsonFileStore(str(state_path)),
            flush_cursor_on_exit=True,
        ) as second,
        second.listen(EVENT_TYPE, filter={"polygon": POLYGON}) as iterator,
    ):
        time.sleep(0.5)
        _delayed_publish(second, 3, 0.5)
        second_received.append(next(iterator).payload["seq"])

    assert second_received == [3], "flush_cursor_on_exit should prevent replay of [1, 2]"
