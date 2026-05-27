"""Discover what event types the server publishes and inspect one schema.

Useful as a first step before publishing or listening: read what your
operator has configured, then build your filter / identifier dicts
against the actual schema rather than guessing.

Expected output (list and field set depend on your server; the example
below is what the local e2e stack returns):

    event types: ['test_polygon', 'test_event']
    test_polygon schema:
      payload required: True
      identifier:
        date     (optional, DateHandler)
        polygon  (required, PolygonHandler)
        time     (optional, TimeHandler)
"""

from __future__ import annotations

import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

import aviso
from _common import require_env


def main() -> None:
    client = aviso.AvisoClient(base_url=require_env(), auth=aviso.Env())

    catalog = client.schema()
    print(f"event types: {catalog.event_types}")

    response = client.schema_for("test_polygon")
    schema = response.schema
    payload_required = schema["payload"]["required"]
    print()
    print("test_polygon schema:")
    print(f"  payload required: {payload_required}")
    print("  identifier:")
    for field, spec in schema["identifier"].items():
        flag = "required" if spec.get("required") else "optional"
        print(f"    {field:8s} ({flag}, {spec.get('type')})")


if __name__ == "__main__":
    main()
