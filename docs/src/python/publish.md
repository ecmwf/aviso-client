# Publishing

Providers publish notifications to announce data or events. You need credentials
with permission to publish to the event type. Publishing sends a notification,
not the file it describes, and does not grant access to that file. Users who
only want to receive notifications can go to [Listening](./listen.md).

## Discover what each event type requires

The server operator owns the schemas. The client can inspect them, but cannot
install one. These examples use the same small `mars` schema as the
[quickstart](./quickstart.md#what-is-on-your-server). Your server may differ.

Start with [pyaviso installed](./install.md) in your Python environment. Set
`AVISO_BASE_URL` and either `AVISO_TOKEN` or `AVISO_USERNAME`/`AVISO_PASSWORD`,
as in [Set the environment](./quickstart.md#set-the-environment).
`pyaviso.Env()` requires credentials and prefers the token when both are set.
For an anonymous server, omit `auth=pyaviso.Env()` from the client
initialization.

Save this as `discover.py` and run `python discover.py`. It lists event types
and prints the complete schema response for `mars`. If `mars` is absent, use a
listed name and run again:

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

With only this example schema installed, output looks like this:

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

`class` must be `od` or `rd`; `EnumHandler` means a choice from a list. `step`
is a whole number (`IntHandler`), with no range limit here. **Publishing needs
every identifier field**, including `step`. The identifier's `required: false`
means it may be omitted from a listener filter, not from a notification. The
payload is optional for this schema. Listing a schema does not prove you have
permission to publish to it.

## A complete publish script

Save this as `publish.py` and run `python publish.py` in the same terminal. It
announces an operational forecast (`class=od`) at step 12:

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

On acceptance it prints:

```text
success
```

`notify` takes keyword arguments: `event_type` names the schema, `identifier`
supplies its labels, and `payload` carries extra information. Pass Python
numbers and lists directly; `12` becomes a JSON number automatically. The
Python keyword is `payload`, not `data`.

The server normalizes scalar identifiers in received notifications: this
`step` comes back as the string `"12"`. Payload numbers keep their JSON types.

The returned `NotifyResponse` has three string properties: `status`,
`request_id`, and `processed_at`. The request ID is for tracing the HTTP request
in server logs, not a notification ID or replay position. `processed_at` is the
server's timestamp. Success means the server accepted the notification, not
that a user has received it or downloaded the file.

## Publishing many notifications at once

To announce several datasets, put their notifications in a list and pass it to
`notify_many`. Here we announce forecasts for steps 24 and 48. In `publish.py`,
replace the `response = client.notify(...)` call and its print with this block,
keeping the imports and client initialization:

```python
notifications = [
    {
        "event_type": "mars",
        "identifier": {"class": "od", "step": 24},
        "payload": {"location": "file:///data/forecast-24.grib"},
    },
    {
        "event_type": "mars",
        "identifier": {"class": "od", "step": 48},
        "payload": {"location": "file:///data/forecast-48.grib"},
    },
]

results = client.notify_many(notifications)

for result in results:
    if result.response is not None:
        print("Notification accepted")
    else:
        print("Notification failed:", result.error)
```

Each notification has its own result. Check for failures even if others
succeeded. Results are returned in the same order as the input list.

Each input is a dict using the same keys as `notify`. The schema still decides
which fields must be supplied. Results are in input order; `index` starts at
zero. `ok` is a boolean. On success, `response` is a `NotifyResponse` and
`error` is `None`; on failure, `response` is `None` and `error` holds the
exception.

The batch is not atomic: valid notifications can be stored even if another
fails. Check the outcome before retrying. Malformed batch input, such as an
item missing `event_type`, raises `ValueError` before sending any requests.
A non-dict item raises `TypeError`.
The optional `concurrency` argument limits how many notifications are sent at
the same time. Omitting it or passing `0` allows up to **16 requests at a time**.

The batch uses different `step` values from the single notification. On a
backend that keeps only the latest notification per subject, publishing the
same routing identifiers again replaces the retained record. Replay reads
retained history, not every publish ever sent.

## Error paths

For the `mars` schema above, leaving out `step` is an error even though it is
optional in filters. This intentionally invalid example keeps the imports and
client initialization from `publish.py`:

```python
try:
    client.notify(event_type="mars", identifier={"class": "od"})
except pyaviso.HttpError as error:
    print(f"server rejected: status={error.status} request_id={error.request_id}")
    print(error.body)
```

The server rejects the missing identifier with HTTP 400. Use its error body to
find the field to fix. An unknown event type, an unsupported identifier, or a
value outside the schema's choices can also be rejected by the server. The
client does not decide which identifier names your server allows.

- `pyaviso.HttpError` means a non-2xx HTTP response. `status` is an integer,
  `body` is a string, and `request_id` is a string or `None` when unavailable.
- `pyaviso.AuthError` means the auth source could not produce credentials.
  `pyaviso.Env()` can raise it during setup, before `notify` is called.
- `pyaviso.TransportError` covers connection or response-transfer failures.
  A failure does not prove the server received nothing.

Publishes are not automatically retried after transport failures: the server
may already have stored the notification. Check before resending to avoid
duplicates. After HTTP 401, a client with an auth provider attempts credential
refresh and retries once. See [Authentication](./auth.md) for credential setup.

## Payload shape

The `mars` payload is optional. To publish without one, keep the imports and
client initialization from `publish.py` and use:

```python
response = client.notify(
    event_type="mars", identifier={"class": "rd", "step": 60}
)
print(response.status)
```

`payload` accepts JSON-compatible Python values, including nested dicts and
lists, strings, numbers, and booleans. `None` means no payload in `notify`.
The server schema determines whether a payload is required. Put extra details
in the payload, rather than inventing identifier fields. Identifiers must
match their handlers; arbitrary objects are not valid values for the `mars`
enum and integer fields.

## Identifier shapes

Spatial examples need a different schema. The following publish assumes the
operator has installed the public server
[observations point-cloud schema](https://sites.ecmwf.int/docs/aviso-server/main/practical-examples/point-cloud-filtering.html#schema).
It requires `date` (`DateHandler`, format `%Y%m%d`), `point_cloud`
(`PointCloudHandler`, at most 10,000 points), and a payload. Inspect it with
`client.schema_for("observations")` before using this example.

Keep the imports and client initialization from `publish.py`:

```python
response = client.notify(
    event_type="observations",
    identifier={
        "date": "20260601",
        "point_cloud": [[46.0, 8.0], [47.0, 9.0]],
    },
    payload={"source": "stations"},
)
print(response.status)
```

A point is `[latitude, longitude]`. Point clouds are lists of those pairs and
accept arrays only. Pass Python lists, not `json.dumps(...)` strings. Clouds
do not need a closing repeat; duplicate points are allowed and keep their
order.

For this schema, subscribers filter with `date` and `polygon`, not
`point_cloud`. A polygon is a list of at least four coordinate pairs, with the
first pair repeated last. Any cloud point inside or on the boundary matches.
See [spatial filters](../cli/publish-and-listen.md#spatial-schema-assumptions).
The reserved `point` field is a watch/replay filter for polygon streams, not a
provider identifier. The
[alternative coordinate format](../cli/publish-and-listen.md#alternative-coordinate-format)
applies to points and polygons, not clouds.

## Admin operations

Operators can use `wipe_stream(event_type)` or `wipe_all()` to clear retained
notifications, and `delete_notification(notification_id)` to remove one by ID.
These are destructive operations requiring the appropriate permissions. A
publish response's `request_id` is not the notification ID to delete. See
[the API reference](./api-reference.md) for the method signatures.

## Async equivalent

If your application uses `asyncio`, the async client takes the same arguments:

```python
import asyncio
import os

import pyaviso


async def main() -> None:
    client = pyaviso.AsyncAvisoClient(
        base_url=os.environ["AVISO_BASE_URL"], auth=pyaviso.Env()
    )
    response = await client.notify(
        event_type="mars",
        identifier={"class": "rd", "step": 72},
        payload={"location": "file:///data/forecast-72.grib"},
    )
    print(response.status)


asyncio.run(main())
```

This uses the same `mars` schema and prints `success` on acceptance. See
[Async](./async.md) for concurrent publishing and listening.
