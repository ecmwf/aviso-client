# Publishing

`AvisoClient.notify(...)` and `AsyncAvisoClient.notify(...)` publish one notification to `aviso-server`. Both return a `NotifyResponse` carrying the server's `request_id` for support correlation.

## Sync

```python
import aviso

client = aviso.AvisoClient(
    base_url="https://aviso.example.org",
    auth=aviso.Bearer("opaque-jwt"),
)

response = client.notify(
    event_type="mars",
    identifier={
        "class": "od",
        "stream": "oper",
        "date": "20260601",
        "expver": "0001",
    },
    payload={"location": "s3://bucket/key"},
)

assert response.status == "success"
print(response.request_id, response.processed_at)
```

## Async

```python
import asyncio
import aviso

async def publish_one() -> aviso.NotifyResponse:
    client = aviso.AsyncAvisoClient(
        base_url="https://aviso.example.org",
        auth=aviso.Bearer("opaque-jwt"),
    )
    return await client.notify(
        event_type="mars",
        identifier={"class": "od", "stream": "oper"},
        payload={"location": "s3://bucket/key"},
    )
```

## Bulk publishing

There is no batch API. Publish in a loop:

```python
for record in records:
    client.notify(
        event_type="mars",
        identifier=record["identifier"],
        payload=record["payload"],
    )
```

The async client lets you parallelise with `asyncio.gather`:

```python
import asyncio

await asyncio.gather(*[
    client.notify(event_type="mars", identifier=r["identifier"], payload=r["payload"])
    for r in records
])
```

The HTTP client pool is shared across calls, so concurrent publishes reuse connections.

## Error paths

- `aviso.TransportError` for network failures (DNS, TCP, TLS) before the response begins.
- `aviso.HttpError` for non-success status. The exception carries `.status`, `.body`, `.request_id`.
- `aviso.AuthError` if the configured auth source cannot produce a header.

Publish errors are not retried automatically: a transport failure after the request body has been sent may have been processed by the server, and the client refuses to risk a duplicate publish without a server-side idempotency contract.

## Payload shape

`payload` accepts any JSON-encodable Python value: `dict`, `list`, `str`, `int`, `float`, `bool`, `None`. Nested structures round-trip cleanly. The server enforces the per-event-type schema; the client passes the value through.
