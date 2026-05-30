# Publishing

`AvisoClient.notify(...)` publishes one notification to `aviso-server` and
returns a `NotifyResponse` carrying the server's `request_id` for support
correlation. This page walks through the end-to-end publish flow, then covers
the variations.

The examples on this page use `test_polygon` as the event type. If your server
does not have it configured, replace the event type and identifier fields with
one of your own; the call shape is the same. See
[What is on your server](./quickstart.md#what-is-on-your-server) in the
quickstart for how to discover what is configured.

## A complete publish script

Save as `publish.py` and run it. The script expects `AVISO_BASE_URL` plus either
`AVISO_TOKEN` or `AVISO_USERNAME`/`AVISO_PASSWORD` in the environment.

```python
"""Publish one test_polygon notification and print the result."""

import os
import aviso

client = aviso.AvisoClient(base_url=os.environ["AVISO_BASE_URL"], auth=aviso.Env())

response = client.notify(
    event_type="test_polygon",
    identifier={
        "polygon": "0,0,1,0,1,1,0,0",
        "date": "20260601",
        "time": "1200",
    },
    payload={"location": "s3://example/data.grib", "size_bytes": 1024},
)

assert response.status == "success"
print(f"published: request_id={response.request_id} processed_at={response.processed_at}")
```

Expected output (one line; the UUID and timestamp differ on each run):

```text
published: request_id=06348659-a3bb-45bd-8541-6e49557c1400 processed_at=2026-05-25T09:01:46Z
```

## Discover what each event type requires

Before calling `notify`, ask the server what fields it expects:

```python
import os
import aviso

client = aviso.AvisoClient(base_url=os.environ["AVISO_BASE_URL"], auth=aviso.Env())

response = client.schema_for("test_polygon")
identifier_schema = response.schema["identifier"]

for field, spec in identifier_schema.items():
    flag = "required" if spec.get("required") else "optional"
    print(f"  {field:12s} ({flag}, {spec.get('type')})")
```

Expected output:

```text
  date         (optional, DateHandler)
  polygon      (required, PolygonHandler)
  time         (optional, TimeHandler)
```

A `notify` call has to supply every identifier field the schema defines,
regardless of the field's `required` flag: there is no such thing as a partial
notification, and the schema's identifier section enumerates the complete
identifier. The `required` flag in the schema tells you what a filter or watch
call has to specify; for `test_polygon` the only required-for-filter field is
`polygon`, but a publish still needs all three. When in doubt, publish a test
value and read the error message from the resulting `aviso.HttpError`.

## Publishing many notifications in a loop

There is no batch API. Publish in a loop:

```python
"""Publish a series of test_polygon notifications at one-minute increments."""

import os
import aviso

client = aviso.AvisoClient(base_url=os.environ["AVISO_BASE_URL"], auth=aviso.Env())

for minute in range(5):
    response = client.notify(
        event_type="test_polygon",
        identifier={
            "polygon": "0,0,1,0,1,1,0,0",
            "date": "20260601",
            "time": f"12{minute:02d}",
        },
        payload={"location": f"s3://example/data/{minute}.grib"},
    )
    print(f"published time=12{minute:02d} request_id={response.request_id}")
```

The HTTP client pool is shared across calls, so the second through fifth
publishes reuse the first one's TCP and TLS connection.

## Error paths

Three exceptions cover almost every publish failure:

```python
"""Handle the three common publish error categories."""

import os
import aviso

client = aviso.AvisoClient(base_url=os.environ["AVISO_BASE_URL"], auth=aviso.Env())

try:
    response = client.notify(
        event_type="test_polygon",
        identifier={
            "polygon": "0,0,1,0,1,1,0,0",
            "date": "20260601",
            "time": "1200",
        },
        payload={"location": "s3://example/data.grib"},
    )
    print(f"ok: {response.request_id}")
except aviso.HttpError as e:
    print(f"server rejected: status={e.status} request_id={e.request_id}")
    print(f"  body: {e.body[:200]}")
except aviso.TransportError as e:
    print(f"network failed before response: {e}")
except aviso.AuthError as e:
    print(f"could not produce credentials: {e}")
```

- `aviso.HttpError` for a non-2xx response. The exception carries `.status`,
  `.body`, and `.request_id` so you can log structured diagnostics and correlate
  with server logs.
- `aviso.TransportError` for failures before the response begins (DNS, TCP,
  TLS). The message names the cause.
- `aviso.AuthError` if the configured auth source cannot produce credentials at
  all.

Publish errors are not auto-retried. A transport failure after the request body
has been sent may already have been processed by the server, and the client
refuses to risk a duplicate publish without a server-side idempotency contract.
If you need retries, wrap the call in your own loop with backoff that you
control.

## Payload shape

`payload=` accepts any JSON-encodable Python value: `dict`, `list`, `str`,
`int`, `float`, `bool`, `None`. Nested structures round-trip cleanly. The server
enforces the per-event-type schema; the client passes the value through.

<!-- not-runnable -->
```python
client.notify(
    event_type="test_polygon",
    identifier={"polygon": "0,0,1,0,1,1,0,0", "date": "20260601", "time": "1200"},
    payload={
        "location": "s3://example/data.grib",
        "size_bytes": 4_194_304,
        "checksum": {"algorithm": "sha256", "value": "ab..."},
        "tags": ["weekly", "production"],
    },
)
```

Identifier values must all be strings. Numbers, booleans, and None are rejected
before the request leaves the client (with a clear `TypeError`); the server
itself accepts strings only.

## Admin operations

The same client publishes notifications and runs admin operations. The two
`wipe_*` methods clear a stream or every stream; `delete_notification` removes
one notification by id.

<!-- not-runnable -->
```python
client.wipe_stream("test_polygon")
client.delete_notification("e3d4f7e2-1cad-4f51-8eb1-3a2b8c8f64ad")
```

These are destructive on shared servers and only meaningful for operators. Run
them against a server you own or in a development environment.

## Async equivalent

The async client takes the same arguments and raises the same exceptions:

```python
"""Async equivalent: publish one notification."""

import asyncio
import os
import aviso


async def main() -> None:
    client = aviso.AsyncAvisoClient(base_url=os.environ["AVISO_BASE_URL"], auth=aviso.Env())
    response = await client.notify(
        event_type="test_polygon",
        identifier={
            "polygon": "0,0,1,0,1,1,0,0",
            "date": "20260601",
            "time": "1200",
        },
        payload={"location": "s3://example/data.grib"},
    )
    print(f"ok: {response.request_id}")


asyncio.run(main())
```

The async client is worth the extra ceremony when you have many publishes
pending at once. See [Async](./async.md) for the fan-out pattern and the other
two situations where async helps.
