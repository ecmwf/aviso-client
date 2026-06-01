from __future__ import annotations

import time
from pathlib import Path

import pyaviso
from _helpers import receive_within

POLYGON = "70,0,71,0,71,1,70,0"
EVENT_TYPE = "test_polygon"


def _publish(client: pyaviso.AvisoClient, seq: int) -> None:
    client.notify(
        event_type=EVENT_TYPE,
        identifier={"polygon": POLYGON, "date": "20260608", "time": f"{seq:04d}"},
        payload={"seq": seq},
    )


def test_flush_cursor_on_exit_prevents_replay_after_clean_shutdown(
    base_url: str,
    producer_auth: pyaviso.Basic,
    tmp_path: Path,
) -> None:
    state_path = tmp_path / "state.json"

    first_received: list[int] = []
    with (
        pyaviso.AvisoClient(
            base_url=base_url,
            auth=producer_auth,
            state_store=pyaviso.JsonFileStore(str(state_path)),
            flush_cursor_on_exit=True,
        ) as first,
        first.listen(EVENT_TYPE, filter={"polygon": POLYGON}) as iterator,
    ):
        time.sleep(0.5)
        _publish(first, 1)
        _publish(first, 2)
        for _ in range(2):
            first_received.append(receive_within(iterator, timeout=5).payload["seq"])

    assert first_received == [1, 2]
    assert state_path.exists(), "JsonFileStore must persist on clean shutdown"

    second_received: list[int] = []
    with (
        pyaviso.AvisoClient(
            base_url=base_url,
            auth=producer_auth,
            state_store=pyaviso.JsonFileStore(str(state_path)),
            flush_cursor_on_exit=True,
        ) as second,
        second.listen(EVENT_TYPE, filter={"polygon": POLYGON}) as iterator,
    ):
        time.sleep(0.5)
        _publish(second, 3)
        second_received.append(receive_within(iterator, timeout=5).payload["seq"])

    assert second_received == [3], "flush_cursor_on_exit should prevent replay of [1, 2]"
