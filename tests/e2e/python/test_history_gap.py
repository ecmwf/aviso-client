# (C) Copyright 2024- ECMWF and individual contributors.
#
# This software is licensed under the terms of the Apache Licence Version 2.0
# which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
# In applying this licence, ECMWF does not waive the privileges and immunities
# granted to it by virtue of its status as an intergovernmental organisation nor
# does it submit to any jurisdiction.

from __future__ import annotations

import pyaviso
import pytest

POLYGON = "30,0,31,0,31,1,30,0"
EVENT_TYPE = "test_polygon"
MAX_HISTORICAL_NOTIFICATIONS = 50


def _time_str(seq: int) -> str:
    hours = (seq - 1) // 60
    minutes = (seq - 1) % 60
    return f"{hours:02d}{minutes:02d}"


@pytest.mark.skip(
    reason=(
        "Investigation needed: aviso-server v0.6.2 silently starts replay from the oldest "
        "available sequence when the requested from_id is below the retained range, instead "
        "of emitting the notification_replay_limit_reached signal the client maps to "
        "HistoryGapError. The Rust supervisor's hermetic test "
        "(crates/aviso/src/watch/supervisor/tests/drain_mapping.rs::"
        "drain_frames_mapping_terminates_on_replay_limit_reached_with_history_gap) verifies "
        "the client side handles the signal; the precise server-side trigger conditions "
        "need a focused dig into aviso-server's replay path before this end-to-end test "
        "can be written reliably."
    )
)
def test_replay_from_pruned_sequence_raises_history_gap(
    producer_client: pyaviso.AvisoClient,
) -> None:
    overflow = MAX_HISTORICAL_NOTIFICATIONS * 2 + 1
    last_response = None
    for i in range(1, overflow + 1):
        last_response = producer_client.notify(
            event_type=EVENT_TYPE,
            identifier={
                "polygon": POLYGON,
                "date": "20260605",
                "time": _time_str(i),
            },
            payload={"seq": i},
        )
    assert last_response is not None

    request = pyaviso.WatchRequest.watch_from(EVENT_TYPE, 1).with_filter({"polygon": POLYGON})
    with (
        pytest.raises(pyaviso.HistoryGapError) as exc_info,
        producer_client.listen(request=request) as iterator,
    ):
        next(iterator)
    assert exc_info.value.reason == "replay_limit_reached"
    assert exc_info.value.max_allowed == MAX_HISTORICAL_NOTIFICATIONS
