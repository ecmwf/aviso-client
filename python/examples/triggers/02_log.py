"""Listen with a log trigger writing NDJSON to a file.

The log trigger appends one compact JSON line per notification to a
file. The supervisor opens the file lazily on first dispatch. Useful
when you want a durable audit record of every notification a process
saw, regardless of how the loop body handled them.

Expected output:

    log path: <tmp>/notifications.log
    received 3 notifications; exiting
    log contents:
    {"event_type":"test_polygon","sequence":...,...}
    ...
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
            for _ in break_after(iterator, 3):
                count += 1
        print(f"received {count} notifications; exiting")

        print("log contents:")
        print(log_path.read_text(encoding="utf-8"), end="")


if __name__ == "__main__":
    main()
