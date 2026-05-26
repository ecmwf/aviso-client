"""Listen for notifications and print each one as it arrives.

The simplest possible listener: construct a client, iterate the watch
stream, print each notification. The for-loop runs until the iterator
ends (which it does not in normal operation, so the example uses
``break_after`` to stop after 3 notifications). A real long-running
listener would just `for n in client.listen(...): ...` and rely on
Ctrl+C to stop.

Run this script in one terminal, then run ``basics/01_publish.py`` in
another terminal a few times to see notifications arrive here.

Expected output (one line per matching publish; sequences differ between
servers and advance over time):

    seq=80 identifier={'polygon': '0,0,1,0,1,1,0,0', 'date': '20260601'} ...
    seq=81 identifier=... payload=...
    seq=82 identifier=... payload=...
    received 3 notifications; exiting
"""

from __future__ import annotations

import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

import aviso
from _common import break_after, require_env


def main() -> None:
    client = aviso.AvisoClient(base_url=require_env(), auth=aviso.Env())
    count = 0
    with client.listen("test_polygon", filter={"polygon": "0,0,1,0,1,1,0,0"}) as iterator:
        for n in break_after(iterator, 3):
            print(f"seq={n.sequence} identifier={n.identifier} payload={n.payload}")
            count += 1
    print(f"received {count} notifications; exiting")


if __name__ == "__main__":
    main()
