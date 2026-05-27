"""Construct a webhook trigger (illustration only; not runnable).

This file shows what a webhook trigger looks like; running it against
the placeholder URL would either fail or spam a host you do not own.
The harness skips the listen call; the construction is the lesson.

For the runnable counterpart that spins up an in-process HTTP server
to receive the POST, see advanced/03_webhook_with_local_server.py.

Expected output (no notifications received because we exit before the
loop runs):

    constructed: <Trigger repr>
    To run end-to-end, see advanced/03_webhook_with_local_server.py
"""

# AVISO_EXAMPLE_NOT_RUNNABLE: placeholder webhook URL; see advanced/03 for the runnable variant

from __future__ import annotations

import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

import aviso


def main() -> None:
    body_template = (
        '{"sequence":"{{ notification.sequence }}","event_type":"{{ notification.event_type }}"}'
    )
    trigger = aviso.Trigger.webhook(
        "https://your-collector.example/notify",
        method=aviso.HttpMethod.POST,
        headers={"Authorization": "Bearer {{ env.HOOK_TOKEN }}"},
        body_template=body_template,
    )
    print(f"constructed: {trigger!r}")
    print("To run end-to-end, see advanced/03_webhook_with_local_server.py")


if __name__ == "__main__":
    main()
