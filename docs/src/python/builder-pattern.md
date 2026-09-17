<!--
SPDX-FileCopyrightText: 2026 European Centre for Medium-Range Weather Forecasts (ECMWF)
SPDX-License-Identifier: Apache-2.0
-->

# Builder pattern

Start with the keyword arguments on [Listening](./listen.md). Use a
`WatchRequest` when you want to name a set of listener settings and reuse it.
A builder is simply a series of calls that returns those settings. There is no
final `.build()` call.

These examples use the
[quickstart's small `mars` schema](./quickstart.md#what-is-on-your-server):
`class` is a required filter with choices `od` and `rd`; the whole-number `step`
filter is optional. Providers supply both fields when publishing. The payload
is optional. Check your server's schema if it differs.

## Build a watch request

This runs without a server. Save it as `build_request.py` and run
`python build_request.py`:

```python
import pyaviso

request = pyaviso.WatchRequest.watch("mars").with_filter({"class": "od"})
print(request.event_type, request.mode)
```

Output:

```text
mars watch
```

The request describes a listener; constructing it does not connect to a server.
Pass it to `client.listen(request=request)`, as below.

## A complete example

Use the installation and
[environment setup](./quickstart.md#set-the-environment)
from the quickstart, including `AVISO_BASE_URL` and credentials for
`pyaviso.Env()`. For an anonymous server, omit `auth=pyaviso.Env()` from the
client initialization. Save this as `listen_builder.py` and run
`python listen_builder.py`:

```python
import os

import pyaviso

client = pyaviso.AvisoClient(
    base_url=os.environ["AVISO_BASE_URL"], auth=pyaviso.Env()
)
request = pyaviso.WatchRequest.watch("mars").with_filter({"class": "od"})

try:
    with client.listen(request=request) as notifications:
        for notification in notifications:
            print(notification)
except KeyboardInterrupt:
    print("Stopped listening")
```

It waits for new operational forecasts at any step and prints each original
CloudEvent as indented JSON, as shown in
[Listening](./listen.md#a-complete-listener).
Press Ctrl+C to stop. You do not need publishing permission to listen. For a
local trial with provider credentials, run the
[publish script](./publish.md#a-complete-publish-script) in another terminal.

`request=` cannot be combined with `event_type`, `filter`, `start_from`, `mode`
or `triggers`. Put those settings on the request instead.

## Replay history and exit

In `listen_builder.py`, replace the `request = ...` line with this block. Keep
the imports, client initialization and loop:

```python
request = pyaviso.WatchRequest.replay_only("mars", 0).with_filter({"class": "od"})
```

Run the script again. It reads matching retained history, prints each
notification and exits at the server's replay boundary. Empty history prints
nothing. It does not publish anything or wait for new notifications.

An integer start position is exclusive: `0` means everything retained after
sequence zero, not every notification ever published. To read after sequence
1024 and then keep listening, use `WatchRequest.watch_from("mars", 1024)`.
Both start-position factories also accept a UTC timestamp string such as
`"2026-06-01T00:00:00Z"`. See
[start positions](./listen.md#start-from-a-specific-position)
and [State and resume](./state-and-resume.md).

## Build a trigger

You can add triggers to run actions before a notification reaches your loop.
Their factories accept keyword arguments. Chainable setters let you adjust an
existing trigger. This standalone example needs no server:

```python
import pyaviso

trigger = pyaviso.Trigger.echo().retries(2).required(False)
request = (
    pyaviso.WatchRequest.watch("mars")
    .with_filter({"class": "od"})
    .with_triggers([trigger])
)
print(request.event_type, request.mode)
```

It prints `mars watch`. Passing this request to a listener adds an optional
echo action with up to two extra attempts. Echo prints a smaller notification
view; `print(notification)` in your loop prints the original CloudEvent.

The setters are `.retries(n)`, `.required(on)`, `.timeout(seconds)`,
`.fail_fast(on)` and `.label(name)`. Timeout and fail-fast settings affect
command and HTTP triggers; label affects only echo. See
[Triggers](./triggers.md#tunables) for failure behavior and
[the API reference](./api-reference.md#triggers) for factory defaults.

## Setters return a new value

Keep the result of a setter: it returns a new value and does not change the
original. This standalone example builds two independent filters:

```python
import pyaviso

base = pyaviso.WatchRequest.watch("mars")
operational = base.with_filter({"class": "od"})
research = base.with_filter({"class": "rd"})
print(operational.event_type, research.event_type)
```

It prints `mars mars`. A call such as `base.with_filter({"class": "od"})` whose
result you discard leaves `base` unchanged.

## When to use the builder

Use keywords for a single listen call. Use a request for settings you want to
reuse or derive from a common base. `Trigger` and `WatchRequest` work with both
clients; they do not belong to a particular client instance. See
[Async](./async.md) if your application already uses `asyncio`.
