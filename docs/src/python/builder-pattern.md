# Builder pattern

`aviso` constructs two values with a fluent, chainable style: `Trigger` and
`WatchRequest`. Each factory returns a value, and each setter returns a new
value, so you keep chaining until the value describes what you want. There is no
final `.build()` step. You pass the value straight to the client.

The keyword-argument forms on `client.listen(...)` and `Trigger.<kind>(...)` are
the default and the shortest path. Reach for the builder when you want to
construct a watch once and reuse it, build one from configuration, keep it in a
registry, or branch a single base into several variants.

This first example builds a trigger and a request without touching the network,
so you can run it as written:

```python
"""Build a Trigger and a WatchRequest with the fluent API. No server needed."""

import aviso

trigger = aviso.Trigger.echo(label="demo").retries(2).required(False)

request = (
    aviso.WatchRequest.watch("test_polygon")
    .with_filter({"polygon": "0,0,1,0,1,1,0,0"})
    .with_triggers([trigger])
)

print(request.event_type, request.mode)  # test_polygon watch
```

Pass the finished `request` to a client with `client.listen(request=request)`.
The [Listening](./listen.md#reusing-a-watch-request) page shows that call from
end to end.

## Build a trigger

Every trigger starts at one of the six factories (`echo`, `log`, `command`,
`webhook`, `teams`, `post`) and accepts tunables you can pass as keyword
arguments, chain as setters, or mix the two. For a tunable that exists in both
places the result is the same:

```python
import aviso

# These two triggers behave identically; pick whichever reads better.
from_kwargs = aviso.Trigger.echo(retries=3, required=False)
from_setters = aviso.Trigger.echo().retries(3).required(False)
```

The setters are `.retries(n)`, `.required(on)`, `.timeout(seconds)`,
`.fail_fast(on)`, and `.label(name)`. The per-kind factory arguments (URLs,
headers, command strings) live in the [Triggers](./triggers.md) guide and the
[API reference](./api-reference.md#triggers); this page does not repeat them.

## Build a watch request

A `WatchRequest` starts at one of three factories, then takes a filter and a
list of triggers:

```python
import aviso

request = (
    aviso.WatchRequest.watch("test_polygon")
    .with_filter({"polygon": "0,0,1,0,1,1,0,0"})
    .with_triggers([aviso.Trigger.echo()])
)
```

The other two factories fix the start position and the mode in one call:

```python
import aviso

# resume after a sequence; mode stays "watch"
aviso.WatchRequest.watch_from("test_polygon", 1024)

# replay a closed range, then stop; mode becomes "replay_only"
aviso.WatchRequest.replay_only("test_polygon", 1024)
```

See [State and resume](./state-and-resume.md) for what the start position means,
and [Listening](./listen.md#replay-only) for how replay-only ends.

Pass the request with the `request=` keyword. It is mutually exclusive with
`event_type=`, `filter=`, `from_=`, `mode=`, and `triggers=`. Passing the
request and any of those together raises `aviso.AvisoError`, so it stays clear
which surface you meant.

## A complete example

Run these two scripts against the same server. `client.notify(...)` takes
keyword arguments and has no builder, so the publisher uses them directly; the
builder pattern is for the watch side, where the listener builds both its
`WatchRequest` and its `Trigger`.

Save the publisher as `publish.py`. It publishes three notifications and exits:

```python
"""Publish three test_polygon notifications, then exit."""

import os
import aviso

client = aviso.AvisoClient(base_url=os.environ["AVISO_BASE_URL"], auth=aviso.Env())

for i in range(3):
    response = client.notify(
        event_type="test_polygon",
        identifier={
            "polygon": "0,0,1,0,1,1,0,0",
            "date": "20260601",
            "time": f"120{i}",
        },
        payload={"location": f"s3://example/data/{i}.grib"},
    )
    print(f"published {i}: request_id={response.request_id}")
```

Save the listener as `listen_builder.py` and start it first, so it is watching
when the publisher runs:

```python
"""Listen with a WatchRequest and Trigger built through the fluent API."""

import os
import aviso

client = aviso.AvisoClient(base_url=os.environ["AVISO_BASE_URL"], auth=aviso.Env())

request = (
    aviso.WatchRequest.watch("test_polygon")
    .with_filter({"polygon": "0,0,1,0,1,1,0,0"})
    .with_triggers([aviso.Trigger.echo(label="demo")])
)

with client.listen(request=request) as iterator:
    for notification in iterator:
        print(f"seq={notification.sequence} payload={notification.payload}")
```

Each notification produces two lines: one compact-JSON line from the echo
trigger, and one `seq=... payload=...` line from the loop body. The sequence
numbers depend on the server's history, so yours will differ:

```text
seq=1 payload={'location': 's3://example/data/0.grib'}
seq=2 payload={'location': 's3://example/data/1.grib'}
seq=3 payload={'location': 's3://example/data/2.grib'}
```

Stop the listener with Ctrl+C. For the resume, replay-only, and async variants
of the same listener, see [Listening](./listen.md).

## Setters return a new value

Both builders are immutable. A setter never changes the value you call it on; it
returns a new value with the change applied. Two things follow from that.

First, you have to keep the result. A setter call whose return value you discard
does nothing:

```python
import aviso

request = aviso.WatchRequest.watch("test_polygon")
request.with_filter({"polygon": "0,0,1,0,1,1,0,0"})  # discarded: no effect
request = request.with_filter({"polygon": "0,0,1,0,1,1,0,0"})  # kept
```

Second, a shared base is safe to branch. Build the base once, then derive
variants that do not affect each other:

```python
import aviso

base = aviso.WatchRequest.watch("test_polygon").with_triggers(
    [aviso.Trigger.echo()]
)

north = base.with_filter({"polygon": "0,0,1,0,1,1,0,0"})
south = base.with_filter({"polygon": "0,0,-1,0,-1,-1,0,0"})
# base is untouched; north and south are independent requests
```

## When to use the builder

| You want to | Use |
|---|---|
| Make a single listen call | kwargs on `client.listen(...)` |
| Build one config and reuse it across calls | `WatchRequest` |
| Construct a watch from operator config or a registry | `WatchRequest` |
| Branch one base into several filtered variants | `WatchRequest` |
| Tune a trigger inline | kwargs on `Trigger.<kind>(...)` |
| Adjust a trigger handed to you from elsewhere | trigger setters |

The same `Trigger` and `WatchRequest` values work with both `AvisoClient` and
`AsyncAvisoClient`; they are plain values with no client of their own. For a
side-by-side builder-versus-kwargs script, see
[`advanced/01_builder_pattern.py`](https://github.com/ecmwf/aviso-client/tree/main/python/examples/advanced/01_builder_pattern.py).
