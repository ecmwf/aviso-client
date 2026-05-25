"""Catch HttpError, TransportError, and the broader AvisoError hierarchy.

Every library-raised exception subclasses ``aviso.AvisoError``. Catch
that to handle anything the library throws; catch a specific subclass
for fine-grained dispatch.

This example deliberately publishes an invalid notification (malformed
polygon) to trigger an HttpError, then prints the structured fields.

Expected output (the request_id changes every run):

    HttpError: status=400 request_id=<uuid>
      details: field 'polygon' must be a valid polygon: ...
"""

from __future__ import annotations

import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

import aviso
from _common import require_env


def main() -> None:
    client = aviso.AvisoClient(base_url=require_env(), auth=aviso.Env())
    try:
        client.notify(
            event_type="test_polygon",
            identifier={
                "polygon": "not-a-polygon",
                "date": "20260601",
                "time": "1200",
            },
            payload={"location": "s3://example/data"},
        )
    except aviso.HttpError as e:
        print(f"HttpError: status={e.status} request_id={e.request_id}")
        print(f"  body: {e.body[:200]}")
    except aviso.TransportError as e:
        print(f"TransportError before response: {e}")
    except aviso.AvisoError as e:
        print(f"AvisoError ({type(e).__name__}): {e}")


if __name__ == "__main__":
    main()
