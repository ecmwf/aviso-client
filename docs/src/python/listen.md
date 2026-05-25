# Listening

`AvisoClient.listen(...)` returns a `NotificationIterator`: a regular Python iterator that yields one `Notification` at a time as the server delivers them. Iterate it with `for`. Press Ctrl+C to stop. The underlying supervisor handles reconnects, checkpoints, and the watch protocol.

## A complete listener

Save as `listen.py` and run it. In a second terminal, publish notifications with the script from the [Publishing page](./publish.md); the listener prints each one as it arrives.

```python
"""Listen for test_polygon notifications and print each one as it arrives.

Press Ctrl+C to stop. The iterator polls every 100 ms and checks for
pending Python signals between polls, so Ctrl+C responds within ~100 ms.
"""

import os
import aviso

client = aviso.AvisoClient(base_url=os.environ["AVISO_BASE_URL"], auth=aviso.Env())

for notification in client.listen("test_polygon", filter={"polygon": "0,0,1,0,1,1,0,0"}):
    print(
        f"seq={notification.sequence} "
        f"date={notification.identifier.get('date')} "
        f"time={notification.identifier.get('time')} "
        f"payload={notification.payload}"
    )
```

Expected output (one line per matching publish; sequences differ between servers and change as the stream advances):

```text
seq=79 date=20260601 time=1200 payload={'location': 's3://example/data/0.grib'}
seq=80 date=20260601 time=1201 payload={'location': 's3://example/data/1.grib'}
seq=81 date=20260601 time=1202 payload={'location': 's3://example/data/2.grib'}
```

`sequence` is monotonic across the stream. `identifier` is the per-notification identifier dict the publisher sent. `payload` is whatever JSON the publisher attached.

## Filtering

`filter=` is a dict of identifier predicates. Each key matches an identifier field; each value is either a string for an exact match or a JSON object for a richer constraint that the field's handler interprets.

```python
"""Listen for test_polygon notifications on a specific polygon and date."""

import os
import aviso

client = aviso.AvisoClient(base_url=os.environ["AVISO_BASE_URL"], auth=aviso.Env())

for notification in client.listen(
    "test_polygon",
    filter={
        "polygon": "0,0,1,0,1,1,0,0",
        "date": "20260601",
    },
):
    print(notification.sequence)
```

Each stream requires its own minimum identifier set. `mars` requires `class`; `test_polygon` requires `polygon`; `dissemination` requires `class` and `destination`. Run `client.schema_for(event_type).schema["identifier"]` to see the full validator list before constructing your filter.

## Resume across restarts

Pass `state_store=` and the supervisor commits the last delivered sequence before sending the next one. Across restarts the iterator picks up at the committed cursor.

```python
"""Listen with a persistent cursor so restarts pick up where we left off."""

import os
import pathlib
import aviso

state_path = pathlib.Path.home() / ".config" / "aviso" / "state.json"
state_path.parent.mkdir(parents=True, exist_ok=True)

client = aviso.AvisoClient(
    base_url=os.environ["AVISO_BASE_URL"],
    auth=aviso.Env(),
    state_store=aviso.JsonFileStore(state_path),
)

for notification in client.listen("test_polygon", filter={"polygon": "0,0,1,0,1,1,0,0"}):
    print(notification.sequence)
```

The first run reads from the live edge. Every subsequent run resumes after the last sequence the supervisor committed. See [State and resume](./state-and-resume.md) for the commit policy and the local-filesystem requirement.

## Start from a specific position

`from_=` accepts an integer sequence or a date-shaped string. The integer form resumes after the given sequence; the string form bootstraps with a date cursor and converts to a sequence cursor after the first commit.

```python
"""Listen starting just after sequence 100."""

import os
import aviso

client = aviso.AvisoClient(base_url=os.environ["AVISO_BASE_URL"], auth=aviso.Env())

for notification in client.listen(
    "test_polygon",
    filter={"polygon": "0,0,1,0,1,1,0,0"},
    from_=100,
):
    print(notification.sequence)
```

For a date-shaped start, pass an ISO 8601 string in UTC:

<!-- not-runnable -->
```python
for notification in client.listen(
    "test_polygon",
    filter={"polygon": "0,0,1,0,1,1,0,0"},
    from_="2026-06-01T00:00:00Z",
):
    print(notification.sequence)
```

## Replay only

`mode="replay_only"` terminates the iterator after the server emits its `replay_completed` control event. Use it for backfill scripts that should not keep listening forever.

<!-- not-runnable -->
```python
"""Backfill from sequence 100 up to the live edge, then exit."""

import os
import aviso

client = aviso.AvisoClient(base_url=os.environ["AVISO_BASE_URL"], auth=aviso.Env())

count = 0
for notification in client.listen(
    "test_polygon",
    filter={"polygon": "0,0,1,0,1,1,0,0"},
    from_=100,
    mode="replay_only",
):
    count += 1
print(f"replayed {count} notifications")
```

The iterator raises `StopIteration` cleanly on the control-event boundary.

## Low-level `WatchRequest`

When you want to build a request once and reuse it, or you want to attach triggers, use `WatchRequest` directly:

```python
"""Build a WatchRequest with triggers and pass it to listen()."""

import os
import aviso

client = aviso.AvisoClient(base_url=os.environ["AVISO_BASE_URL"], auth=aviso.Env())

request = (
    aviso.WatchRequest.watch("test_polygon")
    .with_filter({"polygon": "0,0,1,0,1,1,0,0"})
    .with_triggers([aviso.Trigger.echo()])
)

for notification in client.listen(request=request):
    print(notification.sequence)
```

Mixing `request=` with `event_type=` / `filter=` / `from_=` raises `aviso.AvisoError` so it is always clear which surface you meant. Triggers can only be attached via `WatchRequest.with_triggers(...)`.

## Explicit close

When `flush_cursor_on_exit=True` is set on the client, call `iter.close()` so the supervisor's final cursor flush lands before the process exits:

```python
"""Listen with an explicit close so the last sequence is committed before exit."""

import os
import pathlib
import aviso

state_path = pathlib.Path.home() / ".config" / "aviso" / "state.json"
state_path.parent.mkdir(parents=True, exist_ok=True)

client = aviso.AvisoClient(
    base_url=os.environ["AVISO_BASE_URL"],
    auth=aviso.Env(),
    state_store=aviso.JsonFileStore(state_path),
    flush_cursor_on_exit=True,
)

iterator = client.listen("test_polygon", filter={"polygon": "0,0,1,0,1,1,0,0"})
try:
    for notification in iterator:
        print(notification.sequence)
finally:
    iterator.close()
```

Without an explicit close the supervisor still cancels via iterator drop, but the final commit may not land before the process exits. The default (no flag) is fine for normal at-least-once usage.

## Async equivalent

The async client yields `Notification` instances from `async for`:

```python
"""Async equivalent: listen for test_polygon notifications."""

import asyncio
import os
import aviso


async def main() -> None:
    client = aviso.AsyncAvisoClient(base_url=os.environ["AVISO_BASE_URL"], auth=aviso.Env())
    async for notification in client.listen("test_polygon", filter={"polygon": "0,0,1,0,1,1,0,0"}):
        print(notification.sequence)


asyncio.run(main())
```

`asyncio.run`'s default signal handling delivers `KeyboardInterrupt` to the awaiting task. The iterator drops, the supervisor exits cleanly. See [Async](./async.md) for the situations where the async client actually helps, including draining several streams concurrently from one process.
