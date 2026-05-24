# Listening

`AvisoClient.listen(...)` returns a `NotificationIterator`; `AsyncAvisoClient.listen(...)` returns an `AsyncNotificationIterator`. Both surfaces drain the same underlying supervisor and share the same delivery, reconnect, and checkpoint behaviour as the Rust CLI.

## Sync iteration

```python
import aviso

client = aviso.AvisoClient(base_url="https://aviso.example.org")

for notification in client.listen("mars", filter={"class": "od"}):
    print(notification.sequence, notification.payload)
```

Press `Ctrl+C` to stop. The iterator polls the underlying channel every 100 ms and checks for pending Python signals between polls, so an idle stream still responds to interrupts within one poll period.

## Async iteration

```python
import asyncio
import aviso

async def main() -> None:
    client = aviso.AsyncAvisoClient(base_url="https://aviso.example.org")
    async for notification in client.listen("mars"):
        print(notification.sequence)

asyncio.run(main())
```

`asyncio.run`'s standard signal handling delivers `KeyboardInterrupt` to the awaiting task; the iterator is dropped and the supervisor exits cleanly.

## Filtering

`filter=` accepts a dict of identifier predicates. Values can be strings for scalar matches or JSON objects for spatial / range constraints:

```python
client.listen("mars", filter={
    "class": "od",
    "stream": "oper",
    "date": "20260601",
})
```

## Resume

Pass a `state_store=` to the client and pick up across restarts:

```python
import aviso

client = aviso.AvisoClient(
    base_url="https://aviso.example.org",
    state_store=aviso.JsonFileStore("~/.config/aviso/state.json"),
)

for notification in client.listen("mars"):
    ...
```

The supervisor commits the last delivered notification's sequence before sending the next one. After a restart, iteration picks up at the committed cursor.

## Explicit from-position

```python
for notification in client.listen("mars", from_=42):
    ...

for notification in client.listen("mars", from_="2026-01-01T00:00:00Z"):
    ...
```

`from_=` accepts an integer (treated as `from_id`, resuming after the given sequence) or a string (treated as `from_date`, bootstrapping with a date cursor that converts to a sequence cursor after the first committed notification).

## Replay only

```python
for notification in client.listen("mars", from_=100, mode="replay_only"):
    ...
```

The replay-only mode terminates after the server emits its `replay_completed` control event followed by `end_of_stream`. It is the right shape for backfill scripts that should not keep listening.

## Low-level `WatchRequest`

Build a request once and reuse it:

```python
req = aviso.WatchRequest.watch("mars").with_filter({"class": "od"})

for notification in client.listen(request=req):
    ...
```

Mixing `request=` with the high-level kwargs (`event_type`, `filter`, `from_`, `triggers`) raises `aviso.AvisoError` so it is always clear which surface the caller meant.

## Explicit close

When you configure `flush_cursor_on_exit=True`, call `iter.close()` (sync) or `await iter.aclose()` (async) so the supervisor's final checkpoint flush lands before the process exits:

```python
client = aviso.AvisoClient(
    base_url="https://aviso.example.org",
    state_store=aviso.JsonFileStore("~/.config/aviso/state.json"),
    flush_cursor_on_exit=True,
)

iterator = client.listen("mars")
try:
    for notification in iterator:
        process(notification)
finally:
    iterator.close()
```

Without explicit close the supervisor still cancels via iterator drop, but the final commit may not land before the process exits.
