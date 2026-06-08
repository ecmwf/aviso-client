# (C) Copyright 2024- ECMWF and individual contributors.
#
# This software is licensed under the terms of the Apache Licence Version 2.0
# which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
# In applying this licence, ECMWF does not waive the privileges and immunities
# granted to it by virtue of its status as an intergovernmental organisation nor
# does it submit to any jurisdiction.

"""Listen with a JsonFileStore so restarts pick up where they left off.

The state store remembers the last sequence the supervisor committed.
A long-running process points it at a persistent path
(e.g. ``~/.config/myapp/aviso-state.json``) so a fresh start after a
crash or restart resumes after the committed cursor and replays
nothing it has already seen. ``flush_cursor_on_exit=True`` plus the
iterator's ``with`` form commits the final notification before the
process exits, so a clean Ctrl+C does not cause a replay on next start.

This example uses a temporary directory for the state file (gone when
the script exits), so it shows the API surface rather than actual
resume-across-restart behaviour. For an end-to-end check that resume
works across processes see
``tests/e2e/python/test_resume_across_restart.py``.

Run basics/01_publish.py from another terminal while this listener is
subscribed; its 3-publish window covers the SSE handshake.

Expected output (the hash key inside ``checkpoints`` is derived from
the resume key the watch built; sequence numbers vary):

    state file: <tmp>/state.json
    received 3 notifications; exiting
    state file contents: {'version': 1, 'key_format_version': 1, 'checkpoints':
      {'<hex-hash>': {'last_committed_sequence': <N>,
                     'last_event_id': 'test_polygon@<N>'}}}
"""

from __future__ import annotations

import json
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

import pyaviso
from _common import break_after, require_env, temp_dir


def main() -> None:
    with temp_dir() as workdir:
        state_path = workdir / "state.json"
        print(f"state file: {state_path}")

        client = pyaviso.AvisoClient(
            base_url=require_env(),
            auth=pyaviso.Env(),
            state_store=pyaviso.JsonFileStore(state_path),
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
