"""Runnable webhook trigger backed by an in-process HTTP server.

The webhook trigger POSTs to a URL per notification. Real deployments
point this at a teammate's service, a Microsoft Teams adaptive-card
endpoint, or your own collector. For this example the script stands up
an in-process ``http.server.HTTPServer`` on a random port so the demo
is self-contained: a background thread publishes one notification, the
listener fires the webhook, the server records the POST, the script
asserts before exit.

Expected output (the port and body bytes vary on every run):

    server listening on http://127.0.0.1:<port>
    publisher will fire in ~2 s
    received 1 notification; exiting
    server received 1 POST; first body bytes: b'{"event_type":"test_polygon",...}'
"""

from __future__ import annotations

import sys
import threading
import time
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
        # Suppress default access logging so it does not interleave with
        # the example output.
        pass


def publish_after_delay(client: aviso.AvisoClient, delay: float) -> None:
    """Publish one notification after ``delay`` seconds.

    Runs in a background thread so the main thread can subscribe before
    the publish fires. ``delay`` covers the SSE handshake window
    (~100-300 ms in practice); 2 s is generous.
    """
    time.sleep(delay)
    client.notify(
        event_type="test_polygon",
        identifier={
            "polygon": "0,0,1,0,1,1,0,0",
            "date": "20260601",
            "time": "1200",
        },
        payload={"location": "s3://example/data.grib"},
    )


def main() -> None:
    server = HTTPServer(("127.0.0.1", 0), RecordingHandler)
    port = server.server_port
    print(f"server listening on http://127.0.0.1:{port}")

    http_thread = threading.Thread(target=server.serve_forever, daemon=True)
    http_thread.start()

    client = aviso.AvisoClient(base_url=require_env(), auth=aviso.Env())
    publisher_thread = threading.Thread(target=publish_after_delay, args=(client, 2.0), daemon=True)
    publisher_thread.start()
    print("publisher will fire in ~2 s")

    try:
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
        http_thread.join(timeout=2)
        publisher_thread.join(timeout=2)

    bodies = RecordingHandler.received_bodies
    if not bodies:
        sys.stderr.write("server received no POST; the webhook did not fire as expected\n")
        sys.exit(1)
    print(f"server received {len(bodies)} POST; first body bytes: {bodies[0][:120]!r}")


if __name__ == "__main__":
    main()
