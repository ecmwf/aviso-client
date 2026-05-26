"""Publish one notification to aviso-server and print the response.

The simplest possible publisher: construct a client, call notify(), look
at what came back. Substitute your own event type and identifier fields
if your server has different schemas than the test_polygon used here.

Expected output (two lines; the UUID and timestamp differ on every run):

    status=success request_id=<uuid>
    processed_at=<iso 8601>
"""

from __future__ import annotations

import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

import aviso
from _common import require_env


def main() -> None:
    client = aviso.AvisoClient(base_url=require_env(), auth=aviso.Env())
    response = client.notify(
        event_type="test_polygon",
        identifier={
            "polygon": "0,0,1,0,1,1,0,0",
            "date": "20260601",
            "time": "1200",
        },
        payload={"location": "s3://example/data.grib"},
    )
    print(f"status={response.status} request_id={response.request_id}")
    print(f"processed_at={response.processed_at}")


if __name__ == "__main__":
    main()
