# Quickstart

Three end-to-end scripts. Each one runs against an `aviso-server` you point at
with two environment variables. Pick the one that matches what you want to do.

## Set the environment

Every script in this page reads two environment variables: `AVISO_BASE_URL` for
the server URL and one of `AVISO_TOKEN` or `AVISO_USERNAME`/`AVISO_PASSWORD` for
credentials. `pyaviso.Env()` picks up whichever is set.

```bash
export AVISO_BASE_URL=https://aviso.example.org
export AVISO_USERNAME=alice
export AVISO_PASSWORD=wonderland
```

If you have a bearer token instead, `export AVISO_TOKEN=...` works the same way.

## What is on your server

An aviso-server publishes notifications for one or more streams its operator has
configured. Each stream is an "event type" with a schema that names the
identifier fields the stream uses and which of them are required. The Python
client never defines schemas: it discovers and consumes them.

List what your server has configured:

```python
"""List the event types this server publishes."""

import os
import pyaviso

client = pyaviso.AvisoClient(base_url=os.environ["AVISO_BASE_URL"], auth=pyaviso.Env())
print(client.schema().event_types)
```

The output is a list of event-type names. It will look like
`['mars', 'test_polygon']` or whatever your operator has configured. The exact
set depends on your deployment.

Inspect what one stream expects:

```python
"""Print the schema for one event type."""

import json
import os
import pyaviso

client = pyaviso.AvisoClient(base_url=os.environ["AVISO_BASE_URL"], auth=pyaviso.Env())
print(json.dumps(client.schema_for("test_polygon").schema, indent=2))
```

A schema response looks like this (your fields and types will differ):

```json
{
  "payload": {
    "required": true
  },
  "identifier": {
    "polygon": {
      "required": true,
      "type": "PolygonHandler"
    },
    "date": {
      "canonical_format": "%Y%m%d",
      "required": false,
      "type": "DateHandler"
    },
    "time": {
      "required": false,
      "type": "TimeHandler"
    }
  }
}
```

So a `test_polygon` notification has three identifier fields: `polygon` (an
array of `[latitude, longitude]` pairs), `date` (YYYYMMDD), and `time` (HHMM).
A polygon needs at least four pairs, with the first pair repeated last.
Identifier values are JSON values; the handler determines which shapes are
valid. The `payload` is whatever JSON the publisher attached.

**What `"required": true` means.** The `required` flag on each identifier field
says whether a **filter or watch** call must include it. For `test_polygon`, the
only `required: true` field is `polygon`, so a listener can subscribe with just
`{"polygon": [[0,0],[1,0],[1,1],[0,0]]}`. Other fields narrow the match further.
A **notify** call is different: it must supply every identifier field the schema
defines, regardless of the flag, because the schema enumerates the complete
identifier of a notification. Publishing `test_polygon` without `date` or `time`
returns `400 Required field 'date' missing for notify operation`. If a publish
fails with `Required field X missing`, add X and retry.

**The rest of this page uses `test_polygon` as the example event type.** It is
one stream that may exist on your server.

If your server has no `test_polygon` configured, you have two paths:

1. **Use the local stack from this repo.** From a checkout,
   `bash tests/e2e/shared/stack.sh up` brings up an aviso-server with
   `test_polygon` pre-configured (auth required, write role `producer`). See
   [`python/examples/README.md`](https://github.com/ecmwf/aviso-client/tree/main/python/examples#run-the-examples-against-the-local-stack-recommended)
   for the env-var setup. This is the fastest way to try the rest of this page.

2. **Add the schema to your own aviso-server.** If you have access to the
   server's config, paste this snippet into `notification_schema:` and restart:

   ```yaml
   notification_schema:
     test_polygon:
       payload:
         required: true
       topic:
         base: "polygon"
         key_order: ["date", "time"]
       identifier:
         polygon:
           type: PolygonHandler
           required: true
         date:
           type: DateHandler
           canonical_format: "%Y%m%d"
           required: false
         time:
           type: TimeHandler
           required: false
   ```

   The same schema lives in
   [`tests/e2e/aviso-server.config.yaml`](https://github.com/ecmwf/aviso-client/blob/main/tests/e2e/aviso-server.config.yaml).
   After restart, `client.schema().event_types` should include `test_polygon`.

If you do not run the server yourself and cannot reach a server that has
`test_polygon`, substitute your own event type and identifier fields in every
example below. The shape of every call is the same; only the event-type string
and the identifier keys change.

## 1. Publish one notification

Save as `publish.py` and run it.

```python
"""Publish one notification and print the server's request_id."""

import os
import pyaviso

client = pyaviso.AvisoClient(base_url=os.environ["AVISO_BASE_URL"], auth=pyaviso.Env())

response = client.notify(
    event_type="test_polygon",
    identifier={
        "polygon": [[0, 0], [1, 0], [1, 1], [0, 0]],
        "date": "20260601",
        "time": "1200",
    },
    payload={"location": "s3://example/data.grib"},
)

print(f"status={response.status} request_id={response.request_id}")
print(f"processed_at={response.processed_at}")
```

Expected output (one line each; the UUID and timestamp differ on every run):

```text
status=success request_id=06348659-a3bb-45bd-8541-6e49557c1400
processed_at=2026-05-25T09:01:46Z
```

The `status` is always `success` on a 2xx response; anything else raises
`pyaviso.HttpError` before the print line.

## 2. Listen for notifications

Save as `listen.py` and run it. While it runs, run `publish.py` from a second
terminal a few times and watch the listener pick up each notification.

```python
"""Listen for test_polygon notifications and print each one as it arrives.

Press Ctrl+C to stop. The iterator polls every 100 ms and checks for
pending Python signals between polls, so Ctrl+C responds within ~100 ms.
"""

import os
import pyaviso

client = pyaviso.AvisoClient(base_url=os.environ["AVISO_BASE_URL"], auth=pyaviso.Env())

for notification in client.listen(
    "test_polygon", filter={"polygon": [[0, 0], [1, 0], [1, 1], [0, 0]]}
):
    print(f"seq={notification.sequence} time={notification.identifier.get('time')} payload={notification.payload}")
```

Expected output (one line per matching publish; sequences differ between servers
and advance over time):

```text
seq=80 time=1200 payload={'location': 's3://example/data.grib'}
seq=81 time=1201 payload={'location': 's3://example/data.grib'}
```

The `sequence` number increases monotonically across the stream. Filtering by
`polygon` value means the listener only sees notifications whose polygon
matches.

## 3. Resume across restarts

The first two scripts hold no state. If you stop the listener and restart it,
you miss whatever was published in between. Attaching a state store fixes that.

```python
"""Listen for notifications, remembering where we left off across restarts."""

import os
import pathlib
import pyaviso

state_path = pathlib.Path.home() / ".config" / "aviso" / "state.json"
state_path.parent.mkdir(parents=True, exist_ok=True)

client = pyaviso.AvisoClient(
    base_url=os.environ["AVISO_BASE_URL"],
    auth=pyaviso.Env(),
    state_store=pyaviso.JsonFileStore(state_path),
)

for notification in client.listen(
    "test_polygon", filter={"polygon": [[0, 0], [1, 0], [1, 1], [0, 0]]}
):
    print(f"seq={notification.sequence}")
```

The first run reads from the live edge. Subsequent runs pick up after the last
committed sequence. The last delivered notification can repeat if it was not
committed before exit; see [State and resume](./state-and-resume.md).

The file is locked across cooperating processes on local filesystems (ext4, xfs,
apfs, ntfs). See [State and resume](./state-and-resume.md) for the commit
policy.

## Filters

`filter=` is a dict of identifier predicates. A field flagged `"required": true`
in the schema must appear in your filter; the rest are optional and narrow what
you see further. (A notify call has to supply every identifier field defined in
the schema regardless of the flag, but a listener only has to commit to the
required ones; see "What is on your server" above.)

## What about async?

The async equivalent of recipe 2 looks like this:

```python
"""Async listener for test_polygon notifications."""

import asyncio
import os
import pyaviso


async def main() -> None:
    client = pyaviso.AsyncAvisoClient(base_url=os.environ["AVISO_BASE_URL"], auth=pyaviso.Env())
    async for notification in client.listen(
        "test_polygon", filter={"polygon": [[0, 0], [1, 0], [1, 1], [0, 0]]}
    ):
        print(f"seq={notification.sequence}")


asyncio.run(main())
```

If your script is just running aviso, the sync version is what you want. See
[Async](./async.md) for the situations where the async client actually helps.
