# (C) Copyright 2024- ECMWF and individual contributors.
#
# This software is licensed under the terms of the Apache Licence Version 2.0
# which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
# In applying this licence, ECMWF does not waive the privileges and immunities
# granted to it by virtue of its status as an intergovernmental organisation nor
# does it submit to any jurisdiction.

"""Replay history and exit at end-of-stream.

``mode="replay_only"`` with ``start_from=<sequence>`` opens a backfill stream
that terminates automatically once the server reaches end-of-stream.
Use it for one-shot scripts that backfill historical notifications.

``start_from`` semantics: ``start_from=N`` means "deliver everything strictly
after sequence N". So ``start_from=0`` is the sentinel for "from the very
start". To resume from a checkpoint, pass the last sequence you saw.

This example publishes three notifications under a polygon unique to
the script, then replays everything matching that polygon. On a fresh
stack the count is 3. Later runs replace the same subjects with default
latest-per-subject retention; longer retention can replay more records.

Expected output (the count depends on how many prior runs the stream
has accumulated; the polygon filter keeps it scoped to this example):

    publishing 3 notifications to seed the stream
    replaying from sequence 0
    seq=<N>   received
    seq=<N+1> received
    seq=<N+2> received
    replayed 3 notifications; exiting
"""

from __future__ import annotations

import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

import pyaviso
from _common import require_env

# Polygon unique to this example so replay sees only what this script publishes.
EXAMPLE_POLYGON = [[40, 10], [41, 10], [41, 11], [40, 10]]


def main() -> None:
    client = pyaviso.AvisoClient(base_url=require_env(), auth=pyaviso.Env())

    print("publishing 3 notifications to seed the stream")
    for i in range(1, 4):
        client.notify(
            event_type="test_polygon",
            identifier={
                "polygon": EXAMPLE_POLYGON,
                "date": "20260101",
                "time": f"000{i}",
            },
            payload={"location": f"s3://example/backfill-{i}.grib"},
        )

    print("replaying from sequence 0")
    count = 0
    with client.listen(
        "test_polygon",
        filter={"polygon": EXAMPLE_POLYGON},
        start_from=0,
        mode="replay_only",
    ) as iterator:
        for n in iterator:
            print(f"seq={n.sequence} received")
            count += 1
    print(f"replayed {count} notifications; exiting")


if __name__ == "__main__":
    main()
