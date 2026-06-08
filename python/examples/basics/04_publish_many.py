# (C) Copyright 2024- ECMWF and individual contributors.
#
# This software is licensed under the terms of the Apache Licence Version 2.0
# which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
# In applying this licence, ECMWF does not waive the privileges and immunities
# granted to it by virtue of its status as an intergovernmental organisation nor
# does it submit to any jurisdiction.

"""Publish several notifications in one concurrent call with notify_many.

notify_many sends the whole batch with bounded concurrency and returns one
NotifyResult per input, in order. A per-item failure is reported on the result
(ok is False, error is set) rather than raised, so one bad notification does
not sink the rest. Substitute your own event type and identifier fields if your
server has different schemas than the test_polygon used here.

Expected output (one line per notification; the UUID differs on every run):

    [0] ok request_id=<uuid>
    [1] ok request_id=<uuid>
    [2] ok request_id=<uuid>
"""

from __future__ import annotations

import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

import pyaviso
from _common import require_env


def main() -> None:
    client = pyaviso.AvisoClient(base_url=require_env(), auth=pyaviso.Env())
    notifications = [
        {
            "event_type": "test_polygon",
            "identifier": {
                "polygon": "0,0,1,0,1,1,0,0",
                "date": "20260601",
                "time": f"12{i:02d}",
            },
            "payload": {"location": f"s3://example/data-{i}.grib"},
        }
        for i in range(3)
    ]
    results = client.notify_many(notifications)
    for result in results:
        if result.response is not None:
            print(f"[{result.index}] ok request_id={result.response.request_id}")
        else:
            print(f"[{result.index}] failed: {result.error}")


if __name__ == "__main__":
    main()
