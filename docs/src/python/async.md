# Async

The Python package includes two clients: `aviso.AvisoClient` (sync) and
`aviso.AsyncAvisoClient` (async). The two share the same constructor, the same
auth and state-store configuration, the same exception hierarchy, and return the
same value types. The choice between them is about call style, not capability.

This page is about when the async client is worth using, and what the patterns
look like when it is.

The runnable examples on this page use `test_polygon` as the event type. If your
server does not have it configured, replace the event type and identifier fields
with one of your own; the call shape is the same. See
[What is on your server](./quickstart.md#what-is-on-your-server) in the
quickstart for how to discover what is configured.

## Use `AvisoClient` by default

For most aviso users, `AvisoClient` is what you want. Scripts, batch jobs,
one-shot CLI tools, cron entries, notebooks: all of these benefit from
straight-line code with no event loop ceremony. If your first `asyncio.run`
would exist only because of aviso, stop. Use the sync client.

## When async helps

Three situations make the async client worth the extra ceremony.

### 1. You are already inside an event loop

A FastAPI, Starlette, or aiohttp endpoint cannot block the calling thread with
sync HTTP. Use `AsyncAvisoClient` and `await` directly.

<!-- not-runnable -->
```python
from fastapi import FastAPI
import aviso

app = FastAPI()
client = aviso.AsyncAvisoClient(
    base_url="https://aviso.example.org",
    auth=aviso.Env(),
)

@app.post("/publish")
async def publish(identifier: dict, payload: dict) -> dict:
    response = await client.notify(
        event_type="mars",
        identifier=identifier,
        payload=payload,
    )
    return {"request_id": response.request_id}
```

The same applies to a Jupyter notebook with an active event loop (the IPython
kernel) or to any framework that runs your code as a coroutine.

### 2. You want to drain several streams concurrently

Sync iteration blocks the calling thread. If you need to listen on three streams
from one process, sync forces you into threads. Async lets you express it
directly.

```python
"""Listen on two test_polygon shapes at once and tag each notification.

Run alongside a publisher and you will see notifications from both shapes
interleaved on stdout, tagged by the polygon they matched.
"""

import asyncio
import os
import aviso

BASE_URL = os.environ["AVISO_BASE_URL"]


async def main() -> None:
    client = aviso.AsyncAvisoClient(base_url=BASE_URL, auth=aviso.Env())

    async def drain(tag: str, polygon: str) -> None:
        count = 0
        async for n in client.listen("test_polygon", filter={"polygon": polygon}):
            print(f"[{tag}] seq={n.sequence} time={n.identifier.get('time')}")
            count += 1
            if count >= 2:
                return

    await asyncio.gather(
        drain("square-a", "0,0,1,0,1,1,0,0"),
        drain("square-b", "2,2,3,2,3,3,2,2"),
    )


asyncio.run(main())
```

You could do this with two threads, but the async version is shorter, has no
shared-state hazards, and uses a single HTTP connection pool.

### 3. You want to fan out publishes

The shared HTTP client pool reuses connections across calls. `asyncio.gather`
lets you push many publishes in flight at once.

```python
"""Publish a batch of test_polygon notifications concurrently."""

import asyncio
import os
import aviso

BASE_URL = os.environ["AVISO_BASE_URL"]


async def main() -> None:
    client = aviso.AsyncAvisoClient(base_url=BASE_URL, auth=aviso.Env())

    responses = await asyncio.gather(*[
        client.notify(
            event_type="test_polygon",
            identifier={
                "polygon": "0,0,1,0,1,1,0,0",
                "date": "20260601",
                "time": f"12{i:02d}",
            },
            payload={"location": f"s3://example/data/{i}.grib", "n": i},
        )
        for i in range(5)
    ])

    for response in responses:
        print(response.request_id)


asyncio.run(main())
```

The five publishes complete in roughly the time of one round-trip plus the
slowest of them. The serial sync equivalent would take five round-trips.

## What stays the same

The async client is the same shape as the sync one. The methods, parameters,
return types, exceptions, auth providers, and state stores are identical. Only
the calling style changes.

| Sync | Async |
|---|---|
| `client.notify(...)` | `await client.notify(...)` |
| `for n in client.listen(...): ...` | `async for n in client.listen(...): ...` |
| `client.schema()` | `await client.schema()` |
| `client.schema_for(...)` | `await client.schema_for(...)` |
| `client.wipe_stream(...)` | `await client.wipe_stream(...)` |
| `iterator.close()` | `await iterator.aclose()` |

Anywhere a sync method returns `T`, the async equivalent returns `Awaitable[T]`.
Exceptions come from the same `aviso.AvisoError` hierarchy in both cases.

## Mixing the two is a mistake

Do not call sync methods on `AvisoClient` from inside an asyncio event loop. The
sync surface drives the underlying tokio runtime with `block_on`, which blocks
the asyncio thread until the call returns. Other coroutines stop making
progress; timeouts and cancellations queued on the loop do not fire; on a long
enough call the loop stalls visibly.

The only safe way to use the sync client from inside asyncio is to push it onto
a thread:

<!-- not-runnable -->
```python
import asyncio
result = await asyncio.to_thread(
    client.notify,
    event_type="mars",
    identifier={"class": "od", "stream": "oper", "expver": "0001",
                "date": "20260601", "time": "1200", "step": "0", "domain": "g"},
    payload={"location": "s3://example/data.grib"},
)
```

If you are doing this often, switch to `AsyncAvisoClient` and stop fighting the
loop.
