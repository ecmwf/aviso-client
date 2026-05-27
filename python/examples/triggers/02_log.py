"""Listen with a log trigger writing NDJSON to a file.

The log trigger appends one compact JSON line per notification to a
file. The supervisor opens the file lazily on first dispatch and
flushes after every write, so a `tail -f` on the file in another
terminal will see lines appear in real time. Useful when you want a
durable audit record of every notification a process saw, regardless
of how the loop body handled them.

Run this script in one terminal and basics/01_publish.py in another to
exercise it. The publisher publishes three notifications in a 2-second
window to cover the listener's SSE handshake (~100-300 ms); a single
publish that races the handshake reaches zero subscribers.

Expected output (the per-notification line confirms each one was
already written to the log; the final dump at the end is the same
content read back so you can see what was captured):

    log path: <tmp>/notifications.log
    received seq=<N> (written to log)
    received seq=<N+1> (written to log)
    received seq=<N+2> (written to log)
    done; received 3 notifications
    log contents:
    {"event_type":"test_polygon","sequence":<N>,...}
    {"event_type":"test_polygon","sequence":<N+1>,...}
    {"event_type":"test_polygon","sequence":<N+2>,...}
"""

from __future__ import annotations

import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

import aviso
from _common import break_after, require_env, temp_dir


def main() -> None:
    client = aviso.AvisoClient(base_url=require_env(), auth=aviso.Env())
    with temp_dir() as workdir:
        log_path = workdir / "notifications.log"
        print(f"log path: {log_path}")

        count = 0
        with client.listen(
            "test_polygon",
            filter={"polygon": "0,0,1,0,1,1,0,0"},
            triggers=[aviso.Trigger.log(log_path)],
        ) as iterator:
            for n in break_after(iterator, 3):
                count += 1
                print(f"received seq={n.sequence} (written to log)")
        print(f"done; received {count} notifications")

        print("log contents:")
        print(log_path.read_text(encoding="utf-8"), end="")


if __name__ == "__main__":
    main()
