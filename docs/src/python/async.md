<!--
SPDX-FileCopyrightText: 2026 European Centre for Medium-Range Weather Forecasts (ECMWF)
SPDX-License-Identifier: Apache-2.0
-->

# Async

Use `AvisoClient` for an ordinary script or notebook. This page is for
applications already using `asyncio`, Python's way of letting tasks share a
thread while they wait for input or network responses.

## Use `AvisoClient` by default

You do not need async to receive notifications. The
[regular listener](./listen.md#a-complete-listener) is the simplest starting
point. Use `AsyncAvisoClient` when your application already has an event loop,
or you need several listeners to wait concurrently in one thread.

## A complete async listener

Use the [quickstart environment](./quickstart.md#set-the-environment): install
pyaviso, set `AVISO_BASE_URL` and provide credentials for `pyaviso.Env()`.
For an anonymous server, pass `auth=pyaviso.Anonymous()` instead.

The examples use the
[small `mars` schema](./quickstart.md#what-is-on-your-server).
`class` is a required `od`/`rd` filter; `step` is an optional whole-number
filter. Providers supply both fields and may omit the payload. You only need
receiving permission for the listener.

Save this as `listen_async.py` and run `python listen_async.py`:

```python
import asyncio
import os

import pyaviso


async def main() -> None:
    client = pyaviso.AsyncAvisoClient(
        base_url=os.environ["AVISO_BASE_URL"], auth=pyaviso.Env()
    )
    async with client.listen("mars", filter={"class": "od"}) as notifications:
        async for notification in notifications:
            print(notification)


try:
    asyncio.run(main())
except KeyboardInterrupt:
    print("Stopped listening")
```

It prints each matching new notification as indented CloudEvent JSON. Silence
means no matching notification has arrived. For a local trial, leave it running
and run the [provider script](./publish.md#a-complete-publish-script) in another
terminal. Press Ctrl+C to stop.

`listen()` returns an async iterator directly; do not await the `listen()` call.
`async for` waits for each notification. `async with` awaits the iterator's
`aclose()` when the block exits, including on an exception or a `break`.

On Python 3.11 and newer, the default `asyncio.run()` signal handler makes the
first Ctrl+C cancel the main task so its context managers can clean up. On
Python 3.10, `asyncio.run()` cancels remaining tasks during its final cleanup.
The script works on both: `KeyboardInterrupt` is caught outside `asyncio.run()`.
Do not swallow `asyncio.CancelledError` inside your tasks. Cleanup duration
depends on work in progress; it is not a fixed deadline.

<a id="1-you-are-already-inside-an-event-loop"></a>

In a notebook or framework that already runs an event loop, await `main()` from
that environment instead of nesting `asyncio.run()`.

## When async helps

<a id="2-you-want-to-drain-several-streams-concurrently"></a>

To replay operational and research notifications concurrently, replace `main`
in `listen_async.py` with this definition. This example uses Python 3.11 or
newer for `TaskGroup`:

```python
async def main() -> None:
    client = pyaviso.AsyncAvisoClient(
        base_url=os.environ["AVISO_BASE_URL"], auth=pyaviso.Env()
    )

    async def replay(data_class: str) -> None:
        async with client.listen(
            "mars",
            filter={"class": data_class},
            start_from=0,
            mode="replay_only",
        ) as notifications:
            async for notification in notifications:
                print(notification)

    async with asyncio.TaskGroup() as tasks:
        tasks.create_task(replay("od"))
        tasks.create_task(replay("rd"))
```

It prints matching retained notifications and ends after both replays finish.
Empty history prints nothing. Output from the two listeners can interleave.
The task group waits for its tasks; if one fails, it cancels and waits for the
others and raises an exception group. Each listener still closes its iterator.
See [replay limits](./listen.md#replay-only).

<a id="3-you-want-to-fan-out-publishes"></a>

## Concurrent publishing (providers)

Providers with publishing permission can use `await client.notify_many(...)`
for a batch. Save this as `publish_async.py` and run
`python publish_async.py` with the same environment:

```python
import asyncio
import os

import pyaviso


async def main() -> None:
    client = pyaviso.AsyncAvisoClient(
        base_url=os.environ["AVISO_BASE_URL"], auth=pyaviso.Env()
    )
    notifications = [
        {"event_type": "mars", "identifier": {"class": "od", "step": 24}},
        {"event_type": "mars", "identifier": {"class": "rd", "step": 48}},
    ]
    results = await client.notify_many(notifications, concurrency=2)
    for result in results:
        if result.response is not None:
            print("Notification accepted")
        else:
            print("Notification failed:", result.error)


asyncio.run(main())
```

For valid input and permitted access, it prints `Notification accepted` twice.
Both identifiers are supplied; this schema's payload is optional. The batch is
not atomic: some requests can succeed while others fail. Results stay in input
order. See
[batch publishing](./publish.md#publishing-many-notifications-at-once)
before retrying failures.

## Multiple listeners

`AsyncAvisoClient.listen_many()` delivers the notifications of several
listeners through one `async for` loop; see
[Multiple listeners](./listen-many.md#asynchronous-client).

## What stays the same

The clients use the same constructor options, filters, value types and auth
providers. State-store behavior and its
[unfinished-work limits](./state-and-resume.md) also apply to async listeners.

| Task | Sync | Async |
|---|---|---|
| Publish one | `client.notify(...)` | `await client.notify(...)` |
| Publish a batch | `client.notify_many(...)` | `await client.notify_many(...)` |
| Discover schemas | `client.schema()` | `await client.schema()` |
| Inspect one schema | `client.schema_for(...)` | `await client.schema_for(...)` |
| Iterate | `for notification in iterator` | `async for notification in iterator` |
| Close a listener | `iterator.close()` | `await iterator.aclose()` |

HTTP and schema methods, including admin methods, return awaitables on the
async client. `listen()` is the exception: it returns the iterator directly.
The async client itself is not an async context manager; use `async with` on
its listener. Both clients use the same [error types](./error-handling.md).

## Mixing the two is a mistake

Synchronous calls block the thread running your event loop, preventing other
tasks from making progress. Use `AsyncAvisoClient` inside async code. If you
must call existing synchronous code, `asyncio.to_thread()` can move it to a
worker thread. Async also does not make CPU-heavy analysis nonblocking: move
that work out of the event-loop thread.
