# (C) Copyright 2024- ECMWF and individual contributors.
#
# This software is licensed under the terms of the Apache Licence Version 2.0
# which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
# In applying this licence, ECMWF does not waive the privileges and immunities
# granted to it by virtue of its status as an intergovernmental organisation nor
# does it submit to any jurisdiction.

"""Catch HttpError, TransportError, and the broader AvisoError hierarchy.

Every library-raised exception subclasses ``pyaviso.AvisoError``. Catch
that to handle anything the library throws; catch a specific subclass
for fine-grained dispatch.

This example deliberately publishes an invalid notification (malformed
polygon) to trigger an HttpError, then prints the structured fields.

Expected output (the request_id changes every run; body is the
first 200 chars of the server's JSON error response):

    HttpError: status=400 request_id=<uuid>
      body: {"code":"INVALID_NOTIFICATION_REQUEST","details":"field 'polygon' ...
"""

from __future__ import annotations

import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

import pyaviso
from _common import require_env


def main() -> None:
    client = pyaviso.AvisoClient(base_url=require_env(), auth=pyaviso.Env())
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
    except pyaviso.HttpError as e:
        print(f"HttpError: status={e.status} request_id={e.request_id}")
        print(f"  body: {e.body[:200]}")
    except pyaviso.TransportError as e:
        print(f"TransportError before response: {e}")
    except pyaviso.AvisoError as e:
        print(f"AvisoError ({type(e).__name__}): {e}")


if __name__ == "__main__":
    main()
