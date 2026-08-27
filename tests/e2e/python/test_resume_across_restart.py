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

POLYGON = "20,0,21,0,21,1,20,0"
EVENT_TYPE = "test_polygon"
DATE = "20260604"


def _publish(client: pyaviso.AvisoClient, seq: int) -> None:
    client.notify(
        event_type=EVENT_TYPE,
        identifier={"polygon": POLYGON, "date": DATE, "time": f"{seq:04d}"},
        payload={"seq": seq},
    )


def test_resume_picks_up_after_simulated_restart(
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
        ) as first,
        first.listen(
            EVENT_TYPE,
            filter={"polygon": POLYGON, "date": DATE},
            from_=0,
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
    assert state_path.exists(), "JsonFileStore must persist the committed first item"

    second_received: list[int] = []
    with (
        pyaviso.AvisoClient(
            base_url=base_url,
            auth=producer_auth,
            state_store=pyaviso.JsonFileStore(str(state_path)),
        ) as second,
        second.listen(
            EVENT_TYPE,
            filter={"polygon": POLYGON, "date": DATE},
        ) as iterator,
    ):
        _publish(second, 3)
        deadline = time.monotonic() + 10.0
        while 3 not in second_received and time.monotonic() < deadline:
            remaining = max(deadline - time.monotonic(), 0.5)
            second_received.append(receive_within(iterator, timeout=remaining).payload["seq"])

    assert 3 in second_received, f"item 3 must arrive after resume; got {second_received}"
    assert 1 not in second_received, (
        f"item 1 was committed; should never replay; got {second_received}"
    )
    assert len(second_received) <= 2, (
        f"at most one replay per at-least-once contract; got {second_received}"
    )
