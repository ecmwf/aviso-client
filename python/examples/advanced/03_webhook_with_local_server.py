"""Runnable webhook trigger: spins up an in-process HTTP server.

The companion to triggers/05_webhook.py. That file shows the webhook
trigger API shape with a placeholder URL; this file shows the full
integration with a real HTTP server inside the same process.

The local server binds to a random free port (so concurrent runs do
not collide), accepts one POST, records what it received, and stops.
The listener uses ``http://127.0.0.1:<port>/notify`` as the webhook
URL, runs against one published notification, and exits.

Expected output:

    server listening on http://127.0.0.1:<port>
    received 1 notification; exiting
    server received 1 POST; first body bytes: b'{"event_type":"test_polygon",...}'
"""

from __future__ import annotations

import sys
import threading
from http.server import BaseHTTPRequestHandler, HTTPServer
from pathlib import Path
from typing import ClassVar

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

import aviso
from _common import break_after, require_env


class RecordingHandler(BaseHTTPRequestHandler):
    received_bodies: ClassVar[list[bytes]] = []

    def do_POST(self) -> None:
        length = int(self.headers.get("Content-Length", "0"))
        body = self.rfile.read(length)
        RecordingHandler.received_bodies.append(body)
        self.send_response(200)
        self.end_headers()
        self.wfile.write(b"OK")

    def log_message(self, format: str, *args: object) -> None:
        pass


def main() -> None:
    server = HTTPServer(("127.0.0.1", 0), RecordingHandler)
    port = server.server_port
    print(f"server listening on http://127.0.0.1:{port}")

    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()

    try:
        client = aviso.AvisoClient(base_url=require_env(), auth=aviso.Env())
        count = 0
        with client.listen(
            "test_polygon",
            filter={"polygon": "0,0,1,0,1,1,0,0"},
            triggers=[
                aviso.Trigger.webhook(
                    f"http://127.0.0.1:{port}/notify",
                    method=aviso.HttpMethod.POST,
                )
            ],
        ) as iterator:
            for _ in break_after(iterator, 1):
                count += 1
        print(f"received {count} notification; exiting")
    finally:
        server.shutdown()
        thread.join(timeout=2)

    bodies = RecordingHandler.received_bodies
    if not bodies:
        sys.stderr.write("server received no POST; the webhook did not fire as expected\n")
        sys.exit(1)
    print(f"server received {len(bodies)} POST; first body bytes: {bodies[0][:120]!r}")


if __name__ == "__main__":
    main()
