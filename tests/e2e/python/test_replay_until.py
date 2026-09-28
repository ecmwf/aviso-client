# (C) Copyright 2024- ECMWF and individual contributors.
#
# This software is licensed under the terms of the Apache Licence Version 2.0
# which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
# In applying this licence, ECMWF does not waive the privileges and immunities
# granted to it by virtue of its status as an intergovernmental organisation nor
# does it submit to any jurisdiction.

"""A replay with an end point, ``until=``, against the real server."""

from __future__ import annotations

import random
import time
from collections.abc import Iterator

import pyaviso
from _helpers import receive_within

EVENT_TYPE = "test_event"


def _drain(notifications: Iterator[pyaviso.Notification]) -> list[pyaviso.Notification]:
    """Reads a replay to its end within 60 seconds in total."""
    deadline = time.monotonic() + 60.0
    received: list[pyaviso.Notification] = []
    while True:
        remaining = deadline - time.monotonic()
        if remaining <= 0:
            raise TimeoutError("the replay did not end within 60s")
        try:
            notification: pyaviso.Notification = receive_within(notifications, timeout=remaining)
            received.append(notification)
        except StopIteration:
            return received


def _replay_all(
    client: pyaviso.AvisoClient, date: str, start_from: int
) -> list[pyaviso.Notification]:
    with client.listen(
        EVENT_TYPE, filter={"date": date}, start_from=start_from, mode="replay_only"
    ) as notifications:
        received = _drain(notifications)
    return received


def _replay(
    client: pyaviso.AvisoClient, date: str, start_from: int | str, until: int | str
) -> list[int]:
    with client.listen(
        EVENT_TYPE, filter={"date": date}, start_from=start_from, until=until
    ) as notifications:
        received = _drain(notifications)
    return [notification.sequence for notification in received]


def test_replay_stops_at_a_sequence_or_date_end_point(
    producer_client: pyaviso.AvisoClient,
) -> None:
    date = f"{random.randint(2100, 9999)}{random.randint(1, 12):02d}{random.randint(1, 28):02d}"
    # Notifications a previous run left for the same date are excluded by
    # starting every replay after the last of them.
    retained = _replay_all(producer_client, date, 0)
    baseline = max((n.sequence for n in retained), default=0)
    for hour in ("0000", "0600", "1200", "1800"):
        producer_client.notify(event_type=EVENT_TYPE, identifier={"date": date, "time": hour})

    received = _replay_all(producer_client, date, baseline)
    assert len(received) == 4
    sequences = [notification.sequence for notification in received]
    published = []
    for notification in received:
        assert notification.cloudevent is not None
        published.append(notification.cloudevent["time"])

    # A sequence end is inclusive and the start exclusive.
    by_sequence = _replay(producer_client, date, sequences[0], sequences[2])
    assert by_sequence == sequences[1:3]
    # A date end includes the notification published at exactly that time.
    by_date = _replay(producer_client, date, baseline, published[1])
    assert by_date == sequences[:2]
    # An end point past everything stored ends at the last notification.
    open_ended = _replay(producer_client, date, baseline, "2100-01-01T00:00:00Z")
    assert open_ended == sequences
