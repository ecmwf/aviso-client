# (C) Copyright 2024- ECMWF and individual contributors.
#
# This software is licensed under the terms of the Apache Licence Version 2.0
# which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
# In applying this licence, ECMWF does not waive the privileges and immunities
# granted to it by virtue of its status as an intergovernmental organisation nor
# does it submit to any jurisdiction.

"""Build a WatchRequest once and reuse it: the alternative to triggers/01_echo.py.

The kwargs path on client.listen() is the recommended way to attach
triggers (see triggers/01_echo.py for the same scenario in kwarg form).
The builder pattern is for the rare case where you want to construct
the request once, perhaps from configuration, and pass the same request
to several listen() calls or store it in a registry.

Two snippets shown side-by-side in this docstring so you can compare
(both wrap the iterator in `with` so close() runs on exit):

    # kwarg style (recommended):
    with client.listen(
        "test_polygon",
        filter={"polygon": "0,0,1,0,1,1,0,0"},
        triggers=[pyaviso.Trigger.echo()],
    ) as iterator:
        for n in iterator:
            ...

    # builder style (this file):
    request = (
        pyaviso.WatchRequest.watch("test_polygon")
        .with_filter({"polygon": "0,0,1,0,1,1,0,0"})
        .with_triggers([pyaviso.Trigger.echo()])
    )
    with client.listen(request=request) as iterator:
        for n in iterator:
            ...

The two produce identical iterators against the same server. Passing
both triggers= and request= raises pyaviso.AvisoError.

Run this script in one terminal and basics/01_publish.py in another;
the publisher's 3-publish window covers the listener's SSE handshake.

Expected output (same as triggers/01_echo.py):

    {"event_type":"test_polygon",...}
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
    request = (
        pyaviso.WatchRequest.watch("test_polygon")
        .with_filter({"polygon": "0,0,1,0,1,1,0,0"})
        .with_triggers([pyaviso.Trigger.echo()])
    )
    count = 0
    with client.listen(request=request) as iterator:
        for n in break_after(iterator, 3):
            print(f"seq={n.sequence} received", flush=True)
            count += 1
    print(f"received {count} notifications; exiting", flush=True)


if __name__ == "__main__":
    main()
