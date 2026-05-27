"""Listen with a command trigger running a shell command per notification.

The command trigger spawns /bin/sh -c <rendered_command> for each
matching notification. Notification fields are exposed as AVISO_*
environment variables; the command string can reference them with
{{ notification.<dotted.path> }} and {{ env.<NAME> }} templates.

Unix only. On non-Unix platforms the Trigger.command constructor
raises aviso.ConfigError; this example exits gracefully in that case.

Run this script in one terminal and basics/01_publish.py in another;
the publisher's 3-publish window covers the listener's SSE handshake.

Expected output:

    workdir: <tmp>
    received 3 notifications; exiting
    touch files in workdir: 3
      handled_<sequence>.touch
      handled_<sequence>.touch
      handled_<sequence>.touch
"""

from __future__ import annotations

import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

import aviso
from _common import break_after, require_env, temp_dir


def main() -> None:
    if sys.platform == "win32":
        print("Trigger.command is unix-only; skipping on this platform")
        return

    base_url = require_env()
    client = aviso.AvisoClient(base_url=base_url, auth=aviso.Env())

    with temp_dir() as workdir:
        print(f"workdir: {workdir}")
        command = "touch handled_{{ notification.sequence }}.touch"
        try:
            trigger = aviso.Trigger.command(command, working_dir=workdir)
        except aviso.ConfigError as e:
            print(f"Trigger.command unavailable on this platform: {e}")
            return

        count = 0
        with client.listen(
            "test_polygon",
            filter={"polygon": "0,0,1,0,1,1,0,0"},
            triggers=[trigger],
        ) as iterator:
            for _ in break_after(iterator, 3):
                count += 1
        print(f"received {count} notifications; exiting")

        touches = sorted(workdir.glob("handled_*.touch"))
        print(f"touch files in workdir: {len(touches)}")
        for path in touches:
            print(f"  {path.name}")


if __name__ == "__main__":
    main()
