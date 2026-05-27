"""Publish three notifications and print each server response.

Construct a client, call notify() in a small loop, print what came back
per call. Three calls (rather than one) so that listeners started just
before this script have a chance to subscribe past the SSE handshake
window before all the notifications land. Substitute your own event
type and identifier fields if your server has different schemas than
the test_polygon used here.

Expected output (three blocks; UUID and timestamp differ on every run):

    published #1: status=success request_id=<uuid>
                  processed_at=<iso 8601>
    published #2: ...
    published #3: ...
"""

from __future__ import annotations

import sys
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

import aviso
from _common import require_env


def main() -> None:
    client = aviso.AvisoClient(base_url=require_env(), auth=aviso.Env())
    for i in range(1, 4):
        response = client.notify(
            event_type="test_polygon",
            identifier={
                "polygon": "0,0,1,0,1,1,0,0",
                "date": "20260601",
                "time": "1200",
            },
            payload={"location": f"s3://example/data-{i}.grib"},
        )
        print(f"published #{i}: status={response.status} request_id={response.request_id}")
        print(f"              processed_at={response.processed_at}")
        if i < 3:
            time.sleep(1.0)


if __name__ == "__main__":
    main()
