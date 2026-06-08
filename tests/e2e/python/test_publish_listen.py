# (C) Copyright 2024- ECMWF and individual contributors.
#
# This software is licensed under the terms of the Apache Licence Version 2.0
# which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
# In applying this licence, ECMWF does not waive the privileges and immunities
# granted to it by virtue of its status as an intergovernmental organisation nor
# does it submit to any jurisdiction.

from __future__ import annotations

import time

import pyaviso
from _helpers import receive_within

POLYGON = "0,0,1,0,1,1,0,0"
EVENT_TYPE = "test_polygon"


def test_publish_listen_roundtrips_three_notifications(
    producer_client: pyaviso.AvisoClient,
) -> None:
    expected = [1, 2, 3]
    received: list[dict] = []
    with producer_client.listen(EVENT_TYPE, filter={"polygon": POLYGON}) as iterator:
        time.sleep(0.5)
        for n in expected:
            producer_client.notify(
                event_type=EVENT_TYPE,
                identifier={"polygon": POLYGON, "date": "20260601", "time": f"{n:04d}"},
                payload={"seq": n},
            )
        for _ in expected:
            received.append(receive_within(iterator, timeout=5).payload)

    assert [r["seq"] for r in received] == expected
