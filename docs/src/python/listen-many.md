<!--
SPDX-FileCopyrightText: 2026 European Centre for Medium-Range Weather Forecasts (ECMWF)
SPDX-License-Identifier: Apache-2.0
-->

# Multiple listeners

`client.listen_many()` opens several listeners and delivers their
notifications through a single loop. Each listener has a name, an event type
and, optionally, a filter; listeners may use different event types. Each item
the loop yields is a pair of the listener name and the notification:

```python
for name, notification in notifications:
    ...
```

The same method is available on `AvisoClient` and `AsyncAvisoClient`. It does
not require threads or an `asyncio` event loop of the caller's own.

## When to use it

When the notifications of interest differ only in the value of one
identifier key, a single listener is sufficient. The `in` operator accepts
several values, and the loop can distinguish them:

```python
with client.listen("mars", filter={"class": {"in": ["od", "rd"]}}) as notifications:
    for notification in notifications:
        if notification.identifier["class"] == "od":
            ...
```

Use `listen_many` when the listeners need different identifier keys,
different event types, different triggers or different start positions.

## Example

The example uses the
[quickstart environment](./quickstart.md#set-the-environment) and its `mars`
schema: `class` is a required choice of `od` or `rd`, and `step` is an
optional integer.

Save the following as `listen_many.py` and run `python listen_many.py`:

```python
import pyaviso

client = pyaviso.AvisoClient()

listeners = {
    "operational": {"event_type": "mars", "filter": {"class": "od"}},
    "research": {"event_type": "mars", "filter": {"class": "rd", "step": 0}},
}

try:
    with client.listen_many(listeners) as notifications:
        for name, notification in notifications:
            print(name, notification.identifier)
except KeyboardInterrupt:
    print("Stopped listening")
```

The dictionary maps each listener name to the keyword arguments `listen()`
accepts: `event_type`, `filter`, `start_from`, `mode` and `triggers`. The
script waits for new notifications. With three notifications published from
another terminal (`class=od` at step 12, `class=rd` at step 6, and `class=rd`
at step 0), the output is:

```text
operational {'class': 'od', 'step': '12'}
research {'class': 'rd', 'step': '0'}
```

The notification with `class=rd` at step 6 matches neither listener. Press
Ctrl+C to stop. Leaving the `with` block closes every listener.

## Listeners for different event types

Each listener has its own event type. On a server that also defines an
`alerts` event type, with a `region` of `north` or `south`:

```python
listeners = {
    "forecasts": {"event_type": "mars", "filter": {"class": "od"}},
    "alerts": {"event_type": "alerts", "filter": {"region": "north"}},
}

with client.listen_many(listeners) as notifications:
    for name, notification in notifications:
        print(name, notification.event_type, notification.identifier)
```

```text
forecasts mars {'class': 'od', 'step': '12'}
alerts alerts {'region': 'north'}
```

Each event type defines its own filter keys, and the credential in use must
grant read access to every event type listed.

## Function triggers per listener

To process every notification with application code, without writing the
loop, give each listener a `Trigger.function` and call `run()`:

```python
from functools import partial

import pyaviso
from pyaviso import Trigger


def save(notification, folder):
    print("save", notification.identifier, "to", folder)


client = pyaviso.AvisoClient()
listeners = {
    "operational": {
        "event_type": "mars",
        "filter": {"class": "od"},
        "triggers": [Trigger.function(partial(save, folder="/data/od"))],
    },
    "research": {
        "event_type": "mars",
        "filter": {"class": "rd"},
        "triggers": [Trigger.function(partial(save, folder="/data/rd"))],
    },
}

try:
    client.listen_many(listeners).run()
except KeyboardInterrupt:
    print("Stopped listening")
```

With the same three notifications published, the output is:

```text
save {'class': 'od', 'step': '12'} to /data/od
save {'class': 'rd', 'step': '6'} to /data/rd
save {'class': 'rd', 'step': '0'} to /data/rd
```

The function receives the notification as its only argument. Bind any
further arguments with `functools.partial`, as above, or with a `lambda`.
`run()` processes notifications until every listener has ended or the
process is interrupted, then closes all listeners.

Functions run in the calling thread, one notification at a time, in arrival
order. They therefore need no synchronisation and may call blocking code. A
slow function delays every listener; no notification is lost, because the
client reads from the server more slowly. See
[Triggers](./triggers.md#function) for `retries`, `required` and the
interaction with built-in triggers, which always run first.

## Shared options

`start_from` and `mode` passed to `listen_many` apply to every listener that
does not set its own. The following replays the retained history and then
returns:

```python
listeners = {
    "operational": {"event_type": "mars", "filter": {"class": "od"}},
    "alerts": {"event_type": "alerts", "filter": {"region": "north"}},
}

with client.listen_many(listeners, start_from=0, mode="replay_only") as notifications:
    for name, notification in notifications:
        print(name, notification.sequence, notification.identifier)
print("replay finished")
```

After the preceding examples, one possible output is:

```text
alerts 1 {'region': 'north'}
operational 1 {'class': 'od', 'step': '12'}
operational 4 {'class': 'od', 'step': '12'}
operational 5 {'class': 'od', 'step': '12'}
replay finished
```

Sequence numbers are per event type: `mars` sequences 1, 4 and 5 are the
`od` notifications published for the preceding examples, and `alerts`
sequence 1 is the `north` alert. The loop ends when every listener has
ended; a listener that ends earlier stops contributing.

A listener may also be given as a [`WatchRequest`](./builder-pattern.md)
instead of a dictionary. A `WatchRequest` carries its own start position, so
shared options do not apply to it.

## Error handling

A listener can fail, for example because of a misspelt filter key, a missing
read permission, or a replay that exceeds the server's limit. A
`Trigger.function` can raise an exception. The `on_error` argument defines
the response:

| `on_error` | A listener fails | A required function raises |
|---|---|---|
| `"raise"` (default) | Every listener is closed and the error is raised. | Same as for a listener. |
| `"continue"` | That listener stops; the others continue. | That notification is skipped; the listener continues. |
| A function | It is called with `(name, error)`; the others continue unless it raises. | Same as for a listener. |

Functions are required by default. A function created with
`required=False` does not reach `on_error`: its failure is logged, and the
notification is still delivered.

With `"raise"`, the error keeps its type, its message begins with the
listener name, and the `listener` attribute holds the name:

```python
try:
    client.listen_many(listeners).run()
except pyaviso.HttpError as error:
    print(error.listener, error.status)
```

With `"continue"`, each failure is logged as a warning and recorded in
`errors`. In the following example the second listener misspells `class`, so
the server rejects it for omitting a required key:

```python
listeners = {
    "operational": {"event_type": "mars", "filter": {"class": "od"}},
    "typo": {"event_type": "mars", "filter": {"clas": "rd"}},
}

try:
    with client.listen_many(listeners, on_error="continue") as notifications:
        for name, notification in notifications:
            print(name, notification.identifier)
except KeyboardInterrupt:
    print("Stopped listening")

for failure in notifications.errors:
    print(failure.listener, failure.kind.value, type(failure.error).__name__)
```

```text
listener 'typo': http 400 (...): {..."Required field 'class' missing for watch operation"...} (listener stopped; the other listeners continue)
operational {'class': 'od', 'step': '12'}
Stopped listening
typo listener HttpError
```

`failure.kind` is a `pyaviso.ListenFailureKind`: `LISTENER` when a listener
stopped and `TRIGGER` when a function raised. Being a `str` enum, it also
compares equal to `"listener"` and `"trigger"`. The warnings are logged on the
`pyaviso.listen` logger, so they can be routed or silenced with the standard
`logging` configuration.

When `on_error` is a function, it is responsible for reporting, and no warning
is logged:

```python
def report(name, error):
    print(f"{name} failed: {error}")
    if isinstance(error, pyaviso.AuthError):
        raise error  # stop every listener

client.listen_many(listeners, on_error=report).run()
```

With `"continue"` or a function, if every listener fails, the loop raises
`pyaviso.AvisoError`, whose `failures` attribute lists each failure. A script
therefore cannot finish silently when every listener has failed. After the
loop has stopped, for any reason, further iteration ends immediately.

Shared `start_from` and `mode` values are checked even when every listener
sets its own.

Arguments are validated before any listener opens. An unknown key such as
`filtre`, a missing `event_type` or an invalid `on_error` raises immediately,
and no listener is left running.

## Asynchronous client

On `AsyncAvisoClient`, the same method returns an asynchronous iterator.
Functions may be defined with `async def`; they are awaited:

```python
import asyncio

import pyaviso
from pyaviso import Trigger


async def save(notification):
    await asyncio.sleep(0.1)  # stands in for an asynchronous upload or write
    print("saved", notification.identifier)


async def main():
    client = pyaviso.AsyncAvisoClient()
    listeners = {
        "operational": {
            "event_type": "mars",
            "filter": {"class": "od"},
            "triggers": [Trigger.function(save)],
        },
        "alerts": {"event_type": "alerts", "filter": {"region": "north"}},
    }
    async with client.listen_many(listeners) as notifications:
        async for name, notification in notifications:
            print(name, notification.identifier)


try:
    asyncio.run(main())
except KeyboardInterrupt:
    print("Stopped listening")
```

```text
saved {'class': 'od', 'step': '12'}
operational {'class': 'od', 'step': '12'}
alerts {'region': 'north'}
```

`await notifications.run()` replaces the loop. An `on_error` function may
also be defined with `async def`.

## Behaviour

- **Order.** Within one listener, notifications are delivered in the same
  order as with `listen()`. Across listeners the order is not defined:
  listeners are read in turn, so a busy listener cannot delay a quiet one.
- **Saved positions.** With a `state_store`, each listener keeps its own
  position, the same position `listen()` would use for that event type and
  filter. The listener name is not part of it, so renaming a listener keeps
  its position. Two listeners with the same event type and filter share a
  position, and the client logs a warning when this occurs.
- **Resources.** A listener is not a thread. All listeners run on the
  client's existing background threads and share its connections.
