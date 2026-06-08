# (C) Copyright 2024- ECMWF and individual contributors.
#
# This software is licensed under the terms of the Apache Licence Version 2.0
# which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
# In applying this licence, ECMWF does not waive the privileges and immunities
# granted to it by virtue of its status as an intergovernmental organisation nor
# does it submit to any jurisdiction.

"""Listen with two triggers combined: echo (stdout) plus log (file).

The triggers= kwarg accepts any sequence of Trigger instances. The
supervisor dispatches them in declaration order per notification before
the iterator yields to your code. Combining triggers is the normal way
to do "print AND archive" or "log AND notify".

Run this script in one terminal and basics/01_publish.py in another;
the publisher's 3-publish window covers the listener's SSE handshake.

Expected output:

    log path: <tmp>/notifications.log
    {"event_type":"test_polygon","sequence":...,...}
    seq=... received
    ... (3 such pairs total)
    received 3 notifications; exiting
"""

from __future__ import annotations

import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

import pyaviso
from _common import break_after, require_env, temp_dir


def main() -> None:
    client = pyaviso.AvisoClient(base_url=require_env(), auth=pyaviso.Env())
    with temp_dir() as workdir:
        log_path = workdir / "notifications.log"
        print(f"log path: {log_path}", flush=True)

        count = 0
        with client.listen(
            "test_polygon",
            filter={"polygon": "0,0,1,0,1,1,0,0"},
            triggers=[pyaviso.Trigger.echo(), pyaviso.Trigger.log(log_path)],
        ) as iterator:
            for n in break_after(iterator, 3):
                print(f"seq={n.sequence} received", flush=True)
                count += 1
        print(f"received {count} notifications; exiting", flush=True)


if __name__ == "__main__":
    main()
