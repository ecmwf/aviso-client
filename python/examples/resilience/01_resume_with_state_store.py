"""Listen with a JsonFileStore so restarts pick up where we left off.

The state store remembers the last sequence the supervisor committed.
On the first run the iterator starts at the live edge; on every
subsequent run it resumes after the committed cursor and replays
nothing it has already seen. ``flush_cursor_on_exit=True`` plus the
``with`` form on the iterator ensures the final notification is
committed before the process exits, so a clean Ctrl+C does not cause
a replay on next start.

Expected output (the state file's contents are the supervisor's
internal cursor map keyed by the resume-key the watch derived):

    state file: <tmp>/state.json
    received 3 notifications; exiting
    state file contents: {'<resume-key>': {'sequence': <int>, ...}}
"""

from __future__ import annotations

import json
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

import aviso
from _common import break_after, require_env, temp_dir


def main() -> None:
    with temp_dir() as workdir:
        state_path = workdir / "state.json"
        print(f"state file: {state_path}")

        client = aviso.AvisoClient(
            base_url=require_env(),
            auth=aviso.Env(),
            state_store=aviso.JsonFileStore(state_path),
            flush_cursor_on_exit=True,
        )
        with client.listen("test_polygon", filter={"polygon": "0,0,1,0,1,1,0,0"}) as iterator:
            count = 0
            for _ in break_after(iterator, 3):
                count += 1
            print(f"received {count} notifications; exiting")

        if state_path.exists():
            data = json.loads(state_path.read_text(encoding="utf-8"))
            print(f"state file contents: {data}")
        else:
            print("state file not yet written (no notifications committed)")


if __name__ == "__main__":
    main()
