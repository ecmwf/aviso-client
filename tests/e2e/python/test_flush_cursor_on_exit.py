# (C) Copyright 2024- ECMWF and individual contributors.
#
# This software is licensed under the terms of the Apache Licence Version 2.0
# which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
# In applying this licence, ECMWF does not waive the privileges and immunities
# granted to it by virtue of its status as an intergovernmental organisation nor
# does it submit to any jurisdiction.

from __future__ import annotations

import time
from pathlib import Path

import pyaviso
from _helpers import receive_within

POLYGON = "70,0,71,0,71,1,70,0"
EVENT_TYPE = "test_polygon"
DATE = "20260608"


def _publish(client: pyaviso.AvisoClient, seq: int) -> None:
    client.notify(
        event_type=EVENT_TYPE,
        identifier={"polygon": POLYGON, "date": DATE, "time": f"{seq:04d}"},
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
        first.listen(
            EVENT_TYPE,
            filter={"polygon": POLYGON, "date": DATE},
            start_from=0,
        ) as iterator,
    ):
        _publish(first, 1)
        _publish(first, 2)
        deadline = time.monotonic() + 10.0
        while 2 not in first_received and time.monotonic() < deadline:
            remaining = max(deadline - time.monotonic(), 0.5)
            first_received.append(receive_within(iterator, timeout=remaining).payload["seq"])

    assert first_received[0] == 1
    assert first_received[-1] == 2
    assert set(first_received) == {1, 2}
    assert state_path.exists(), "JsonFileStore must persist on clean shutdown"

    second_received: list[int] = []
    with (
        pyaviso.AvisoClient(
            base_url=base_url,
            auth=producer_auth,
            state_store=pyaviso.JsonFileStore(str(state_path)),
            flush_cursor_on_exit=True,
        ) as second,
        second.listen(
            EVENT_TYPE,
            filter={"polygon": POLYGON, "date": DATE},
        ) as iterator,
    ):
        _publish(second, 3)
        second_received.append(receive_within(iterator, timeout=5).payload["seq"])

    assert second_received == [3], "flush_cursor_on_exit should prevent replay of [1, 2]"
