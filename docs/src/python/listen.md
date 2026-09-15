# Listening

Receive notifications from data providers and use their labels to select what
you need. You do not need to publish anything to listen.

Start with [pyaviso installed](./install.md). Set `AVISO_BASE_URL` and
credentials with permission to receive notifications, as in
[Set the environment](./quickstart.md#set-the-environment). The examples use
`pyaviso.Env()`: set `AVISO_TOKEN`, or unset it and set both `AVISO_USERNAME`
and `AVISO_PASSWORD`. For an anonymous server, omit `auth=pyaviso.Env()`
from each client initialization; `Env()` requires credentials.

## A complete listener

These examples use the same small `mars` schema as the
[quickstart](./quickstart.md#what-is-on-your-server) and
[Publishing](./publish.md). It has a required `class` filter (`od` or `rd`) and
an optional whole-number `step` filter. If your server differs, use
[schema discovery](#check-your-servers-schema) below to choose its event type
and fields.

Save this as `listen.py`:

```python
import os

import pyaviso

client = pyaviso.AvisoClient(
    base_url=os.environ["AVISO_BASE_URL"], auth=pyaviso.Env()
)

try:
    with client.listen("mars", filter={"class": "od"}) as notifications:
        for notification in notifications:
            print(
                f"seq={notification.sequence} "
                f"labels={notification.identifier} "
                f"payload={notification.payload}"
            )
except KeyboardInterrupt:
    print("Stopped listening")
```

Run `python listen.py` in the terminal where you set the environment. It waits
for **new** `mars` notifications with `class=od`, at any step. Silence is normal
when nothing matches. For a local trial, start this listener first, then run
the [publish script](./publish.md#a-complete-publish-script) in another terminal
with the same environment. Press Ctrl+C to stop.

That publish produces a line like this (your sequence will differ):

```text
seq=1 labels={'class': 'od', 'step': '12'} payload={'location': 'file:///data/forecast.grib'}
```

`identifier` holds the notification's labels. The server normalizes scalar
labels, so the published integer `step=12` arrives as the string `"12"`.
`payload` holds extra information from the provider, such as a file location;
receiving it does not download the file. It is `None` when absent. `sequence`
is a position used for replay, not a promise of strictly increasing delivery
order or a count of your matches.

This script does not save a cursor to disk. Starting it again starts fresh at
the live edge, rather than reading notifications published while it was off.

## Check your server's schema

Save this as `discover.py` and run `python discover.py`. It lists event types
and prints the schema response for `mars`. If `mars` is absent, replace it with
a listed name and run again. Discovery reads the operator's configuration; it
does not install schemas or prove your access rights.

```python
import json
import os

import pyaviso

client = pyaviso.AvisoClient(
    base_url=os.environ["AVISO_BASE_URL"], auth=pyaviso.Env()
)
print(client.schema().event_types)
print(json.dumps(client.schema_for("mars").as_dict(), indent=2, sort_keys=True))
```

With only the small `mars` schema installed, the output is:

```text
['mars']
{
  "event_type": "mars",
  "schema": {
    "identifier": {
      "class": {
        "required": true,
        "type": "EnumHandler",
        "values": [
          "od",
          "rd"
        ]
      },
      "step": {
        "range": null,
        "required": false,
        "type": "IntHandler"
      }
    },
    "payload": {
      "required": false
    }
  },
  "status": "success"
}
```

`EnumHandler` means a choice from a list; `IntHandler` means a whole number.
Here `step` has no range limit. An identifier's `required: true` means it must
appear in a listener filter. `required: false` lets you omit that filter field.
Providers still supply **every identifier field**, including `step`. The
payload is optional for this schema.

## Filtering

`filter=` selects identifier labels, not payload contents. Start with the
required `class`, as in `listen.py`. To receive only step 12, replace its
`try`/`except` block with this, keeping the imports and client initialization:

```python
with client.listen("mars", filter={"class": "od", "step": 12}) as notifications:
    for notification in notifications:
        print(notification.identifier)
```

Both fields must match. An `rd` notification or an `od` notification at step
24 is excluded. Use Python numbers and lists directly, rather than JSON-encoded
strings. The examples below also keep the imports and client from `listen.py`
unless they show their own setup. Live examples wait until you press Ctrl+C;
without the `try`/`except`, Python also prints a `KeyboardInterrupt` traceback.

## Start from a specific position

Add `from_=` to read retained notifications before continuing with new ones.
An integer starts **after** that sequence; `from_=0` requests retained history
after sequence zero. A UTC date string such as `from_="2026-06-01T00:00:00Z"`
selects publication time, not a `date` field in the notification's labels.
To filter those labels, put `date` in `filter=` if the schema supports it.

## Replay only

Use `mode="replay_only"` with a starting position for a script that finishes:

```python
with client.listen(
    "mars", filter={"class": "od"}, from_=0, mode="replay_only"
) as notifications:
    for notification in notifications:
        print(notification.identifier)
```

If the publish script's step-12 notification is the only matching retained
record, this prints:

```text
{'class': 'od', 'step': '12'}
```

An empty matching history prints nothing. Replay ends at the history boundary
captured at the start, when the server signals `replay_completed`. It does not
wait for new publications. Retention may have removed older records. If the
server truncates replay at its cap, the client raises `pyaviso.HistoryGapError`;
a failed run is not proof of full catch-up. See
[historical replay limits](https://sites.ecmwf.int/docs/aviso-server/main/streaming-semantics.html#historical-replay-limits).

Errors can arise when opening or iterating the listener. For example, a rejected
filter can raise `pyaviso.HttpError`; inspect its `status` and `body`.
Connection losses and retryable server failures normally trigger reconnection.
A quiet listener is not itself an error. See
[error types](./publish.md#error-paths) and
[the API reference](./api-reference.md).

### Numeric and enum constraints

After installing the schema and publishing the five seeds in the
[weather tutorial](../cli/publish-and-listen.md#weather-constraints), point
`AVISO_BASE_URL` at that test server. This replay prints B and C, then ends:

```python
with client.listen(
    "weather",
    filter={
        "date": "20260913",
        "severity": {"gte": 5},
        "anomaly": {"between": [40, 50]},
        "region": {"in": ["north", "south"]},
    },
    from_=0,
    mode="replay_only",
) as notifications:
    for notification in notifications:
        print(notification.payload["id"])
```

`gte` includes 5; `between` includes both endpoints; `in` accepts either listed
region. These dicts are identifier predicates, not payload queries. See the
[shared constraint rules](../concepts/filters.md#constraint-filters).

## Spatial filters

Spatial examples need different schemas. The server's public
[`test_polygon` example](https://github.com/ecmwf/aviso-server/blob/main/configuration/config.yaml.example)
requires a `polygon` filter. Its `date` (`DateHandler`, `%Y%m%d`) and `time`
(`TimeHandler`) are optional in subscriber filters. Providers supply all three
identifiers and a required payload. Inspect it with
`client.schema_for("test_polygon")` before using this replacement listener:

```python
with client.listen(
    "test_polygon",
    filter={"polygon": [[0, 0], [1, 0], [1, 1], [0, 0]], "date": "20260601"},
) as notifications:
    for notification in notifications:
        print(notification.identifier)
```

Coordinates are `[latitude, longitude]`. A polygon needs at least four pairs,
with the first pair repeated last. Pass nested Python lists, not an encoded
string. Spatial identifiers arrive as arrays rather than normalized strings.

The public
[`observations` schema](https://sites.ecmwf.int/docs/aviso-server/main/practical-examples/point-cloud-filtering.html#schema)
works differently: providers send a required `date`, a `point_cloud` array, and
a payload. Subscribers filter with `date` and a closed `polygon`, not
`point_cloud`. Any cloud point inside or on the polygon boundary matches. See
[Publishing: identifier shapes](./publish.md#identifier-shapes) for the paired
provider example.

## Resume across restarts

To save a resume position, replace the client initialization in `listen.py`
with this block. Keep its imports and `try`/`except` listener:

```python
from pathlib import Path

state_path = Path.home() / ".config" / "aviso" / "state.json"
state_path.parent.mkdir(parents=True, exist_ok=True)
client = pyaviso.AvisoClient(
    base_url=os.environ["AVISO_BASE_URL"],
    auth=pyaviso.Env(),
    state_store=pyaviso.JsonFileStore(state_path),
)
```

Use a local filesystem. With no saved cursor, the first run starts at the live
edge. Later runs with the same server, schema and filter resume after the saved
position. The client commits a pending sequence before sending the next
notification to the iterator's buffer. **This is not an acknowledgement that
your loop finished its work.** Make repeated handling safe; at-least-once
redelivery depends on retained history and a usable saved cursor, and does not
guarantee completion of application work. See
[State and resume](./state-and-resume.md) for resume keys and storage details.

## Clean shutdown with `with`

Exiting `with` calls the synchronous iterator's `close()`, cancelling and
waiting for its background task to finish. This also happens after a loop body
raises or breaks. A `break` alone does not close an iterator you still hold;
leave the context or call `close()` explicitly.

`flush_cursor_on_exit` defaults to `False`, leaving the last pending sequence
uncommitted when no later notification arrives. With a state store, setting
`flush_cursor_on_exit=True` on the client attempts to save that pending cursor
during shutdown. Context exit waits for the attempt, but a storage failure can
still prevent it. Flushing does not certify that your application processed
every buffered notification. Without a saved cursor, a fresh run starts live.

## Reusing a watch request

For reusable listener settings, use `WatchRequest` and pass it as `request=`.
It cannot be combined with the event/filter/start/mode/trigger arguments used
above. See [Builder pattern](./builder-pattern.md) for complete examples and
[Triggers](./triggers.md) for actions attached to a listener.

## Async equivalent

For an application using `asyncio`, use `AsyncAvisoClient`, `async with`, and
`async for`. Async context exit awaits `aclose()`, rather than the synchronous
`close()`. With the default `asyncio.run()` signal handler, Ctrl+C cancels the
main task first, allowing context cleanup; `KeyboardInterrupt` is then raised
outside `asyncio.run()`. See [Async](./async.md) for complete listener examples.
