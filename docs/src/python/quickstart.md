# Quickstart

Use Python to receive notifications from data providers. Start by checking what
your server offers, then listen for new notifications or replay past ones. You
do not need to publish anything to listen.

## Set the environment

You need Python 3.10 or newer and [pyaviso installed](./install.md) in your
Python environment. Ask your server operator for a server URL and credentials
with permission to receive notifications.

Set these in your terminal, replacing the example values with yours:

```bash
export AVISO_BASE_URL=https://aviso.example.org
export AVISO_TOKEN=your-bearer-token
```

The scripts use `pyaviso.Env()` to read credentials. If you have a username and
password instead, unset `AVISO_TOKEN` and set `AVISO_USERNAME` and
`AVISO_PASSWORD`. A token takes precedence when both are set. `Env()` requires
credentials; for an anonymous server, omit `auth=pyaviso.Env()` from each client
initialization. See [Authentication](./auth.md) for other options.

## What is on your server

The server operator configures **event types**, each with a schema describing
its notifications. The examples below use the same small `mars` schema as the
[CLI quickstart](../cli/quickstart.md#see-what-the-server-knows). Your server
may have different event types or a different `mars` schema.

Save this as `discover.py` and run `python discover.py` in the terminal where
you set the environment variables. It lists event types, then prints the schema
for `mars`. If `mars` is absent, use a listed name and run again.

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

On a server configured with only this example schema, the output is:

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

The list names configured schemas, not available datasets or access rights.
`as_dict()` makes the schema response printable as JSON.

### Filters

**Identifiers** are labels you can filter on. Here, `class` must be `od` or `rd`
(`EnumHandler` means a choice from a list). In this example, `od` means
operational data. `step` is a whole number (`IntHandler`) for forecast hours;
`range: null` adds no range limit.

`required: true` means a field must appear in a filter. You can leave out `step`
to receive all steps. Publishing is different: a provider must supply **every
identifier field**, including `step`. The optional **payload** carries extra
information, such as a file location, rather than labels to filter on.

Use your server's schema to adapt the event name and filters below. You do not
need to configure a server schema to receive notifications from an existing
service.

<a id="2-listen-for-notifications"></a>

## Listen for notifications

Save this as `listen.py` and run `python listen.py` in the same terminal. It
selects `mars` notifications whose `class` is `od`, at any forecast step:

```python
import os

import pyaviso

client = pyaviso.AvisoClient(
    base_url=os.environ["AVISO_BASE_URL"], auth=pyaviso.Env()
)
for notification in client.listen("mars", filter={"class": "od"}):
    print(notification.payload)
```

The script waits for matching new notifications and prints each payload. Silence
can simply mean none have arrived. For a notification carrying a file location,
you would see:

```text
{'location': 'file:///data/forecast.grib'}
```

Notifications with `class=rd` do not match. Press Ctrl+C to stop. This script
does not save progress: restarting it waits for new notifications again.

## Replay past notifications

To read retained history, replace the `for` loop in `listen.py` with this loop,
keeping the imports and client initialization above it. Run `python listen.py`
again:

```python
for notification in client.listen(
    "mars", filter={"class": "od"}, from_=0, mode="replay_only"
):
    print(notification.payload)
```

`from_=0` starts from the beginning of retained history. `mode="replay_only"`
makes the script end when it catches up, rather than wait for new notifications.
It prints matching payloads in sequence order. No output means no retained
notifications match. Running it again reads the same retained history; it does
not save a position.

<a id="3-resume-across-restarts"></a>

## Resume across restarts

For a listener that remembers its position, see
[State and resume](./state-and-resume.md). For more filters and replay options,
see [Listening](./listen.md).

## What about async?

Use the regular client above for a simple script. If your application already
uses `asyncio`, see [Async](./async.md) for complete examples.

<a id="1-publish-one-notification"></a>

## Publish a notification (optional, for providers)

You need permission to publish. With the schema above, save this as `publish.py`
and run `python publish.py` to announce an operational forecast at step 12:

```python
import os

import pyaviso

client = pyaviso.AvisoClient(
    base_url=os.environ["AVISO_BASE_URL"], auth=pyaviso.Env()
)
response = client.notify(
    event_type="mars",
    identifier={"class": "od", "step": 12},
    payload={"location": "file:///data/forecast.grib"},
)
print(response.status)
```

It prints `success` when the server accepts the notification. Both `class` and
`step` are supplied, even though `step` is optional in filters. The location is
an example reference: publishing sends the notification, not the file, and does
not grant access to the file.

An active matching listener receives this payload. To try it yourself, leave
the live version of `listen.py` running and publish from another terminal with
the same environment setup. A publish sent before the listener connects can be
read with the replay loop. See [Publishing](./publish.md) for more options.
