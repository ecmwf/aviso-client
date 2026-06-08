# (C) Copyright 2024- ECMWF and individual contributors.
#
# This software is licensed under the terms of the Apache Licence Version 2.0
# which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
# In applying this licence, ECMWF does not waive the privileges and immunities
# granted to it by virtue of its status as an intergovernmental organisation nor
# does it submit to any jurisdiction.

"""Listen with an echo trigger attached via the triggers= kwarg.

Each matching notification is printed by the supervisor (the echo
trigger) before the iterator yields it back to your code. The iterator
loop body then sees the same notifications.

Run this script in one terminal and basics/01_publish.py in another to
see the echo lines on stdout. The publisher publishes three notifications
in a 2-second window to cover the listener's SSE handshake (~100-300 ms);
a single publish that races the handshake reaches zero subscribers.

Expected output (one block per matching publish, then the loop print):

    {"event_type":"test_polygon","sequence":...,"identifier":{...},"payload":{...}}
    seq=... received
    received 3 notifications; exiting
"""

from __future__ import annotations

import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

import pyaviso
from _common import break_after, require_env


def main() -> None:
    client = pyaviso.AvisoClient(base_url=require_env(), auth=pyaviso.Env())
    count = 0
    with client.listen(
        "test_polygon",
        filter={"polygon": "0,0,1,0,1,1,0,0"},
        triggers=[pyaviso.Trigger.echo()],
    ) as iterator:
        for n in break_after(iterator, 3):
            print(f"seq={n.sequence} received", flush=True)
            count += 1
    print(f"received {count} notifications; exiting", flush=True)


if __name__ == "__main__":
    main()
