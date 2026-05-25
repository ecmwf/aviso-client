"""Replay history from a known sequence and exit at end-of-stream.

``mode="replay_only"`` with ``from_=<int sequence>`` opens a backfill
stream that terminates after the server emits its replay-complete
control event. Useful for one-shot scripts that should not keep
listening forever.

Choose a from_ value that is plausibly old enough to have notifications
on your server; the example uses 1 to replay from the beginning.

Expected output:

    replaying from sequence 1
    seq=1 received
    seq=2 received
    seq=3 received
    replayed 3 notifications; exiting
"""

from __future__ import annotations

import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

import aviso
from _common import break_after, require_env


def main() -> None:
    client = aviso.AvisoClient(base_url=require_env(), auth=aviso.Env())
    from_sequence = 1
    print(f"replaying from sequence {from_sequence}")
    iterator = client.listen(
        "test_polygon",
        filter={"polygon": "0,0,1,0,1,1,0,0"},
        from_=from_sequence,
        mode="replay_only",
    )
    count = 0
    for n in break_after(iterator, 3):
        print(f"seq={n.sequence} received")
        count += 1
    print(f"replayed {count} notifications; exiting")


if __name__ == "__main__":
    main()
