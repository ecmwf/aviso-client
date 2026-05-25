# Quickstart

Three end-to-end scripts. Each one runs against an `aviso-server` you can point at with two environment variables. Pick the one that matches what you want to do.

## Set the environment

Every script in this page reads two environment variables: `AVISO_BASE_URL` for the server URL and one of `AVISO_TOKEN` or `AVISO_USERNAME`/`AVISO_PASSWORD` for credentials. `aviso.Env()` picks up whichever is set.

```bash
export AVISO_BASE_URL=https://aviso.example.org
export AVISO_USERNAME=alice
export AVISO_PASSWORD=wonderland
```

If you have a bearer token instead, `export AVISO_TOKEN=...` works the same way.

## 1. Publish one notification

Save as `publish.py` and run it.

```python
"""Publish one notification and print the server's request_id."""

import os
import aviso

client = aviso.AvisoClient(base_url=os.environ["AVISO_BASE_URL"], auth=aviso.Env())

response = client.notify(
    event_type="test_polygon",
    identifier={
        "polygon": "0,0,1,0,1,1,0,0",
        "date": "20260601",
        "time": "1200",
    },
    payload={"location": "s3://example/data.grib"},
)

print(f"status={response.status} request_id={response.request_id}")
print(f"processed_at={response.processed_at}")
```

Expected output:

```text
status=success request_id=06348659-a3bb-45bd-8541-6e49557c1400
processed_at=2026-05-25T09:01:46Z
```

Each run prints a different `request_id` and `processed_at`. The `status` is always `success` on a 2xx response; anything else raises `aviso.HttpError` before this line.

## 2. Listen for notifications

Save as `listen.py` and run it. While it runs, run `publish.py` from a second terminal a few times and watch the listener pick up each notification.

```python
"""Listen for test_polygon notifications and print each one as it arrives.

Press Ctrl+C to stop. The iterator polls every 100 ms and checks for
pending Python signals between polls, so Ctrl+C responds within ~100 ms.
"""

import os
import aviso

client = aviso.AvisoClient(base_url=os.environ["AVISO_BASE_URL"], auth=aviso.Env())

for notification in client.listen("test_polygon", filter={"polygon": "0,0,1,0,1,1,0,0"}):
    print(f"seq={notification.sequence} time={notification.identifier.get('time')} payload={notification.payload}")
```

Expected output (one line per matching publish; sequences differ between servers and advance over time):

```text
seq=80 time=1200 payload={'location': 's3://example/data.grib'}
seq=81 time=1201 payload={'location': 's3://example/data.grib'}
```

The `sequence` number increases monotonically across the stream. Filtering by `polygon` value means the listener only sees notifications whose polygon matches.

## 3. Resume across restarts

The first two scripts hold no state. If you stop the listener and restart it, you miss whatever was published in between. Wiring a state store fixes that.

```python
"""Listen for notifications, remembering where we left off across restarts."""

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
    print(f"seq={notification.sequence}")
```

The first run reads from the live edge. Subsequent runs pick up at the last committed sequence, so a Ctrl+C followed by a fresh start replays nothing it has already processed.

The file is locked across cooperating processes on local filesystems (ext4, xfs, apfs, ntfs). See [State and resume](./state-and-resume.md) for the commit policy.

## Filters: what to put in them

`filter=` is a dict of identifier predicates. Each aviso stream has its own required identifier fields. Discover them with `client.schema_for("<event_type>")`:

```python
import os
import aviso

client = aviso.AvisoClient(base_url=os.environ["AVISO_BASE_URL"], auth=aviso.Env())
response = client.schema_for("test_polygon")
print(sorted(response.schema["identifier"].keys()))
```

Expected output:

```text
['date', 'polygon', 'time']
```

A field flagged `"required": true` in the schema must be in your filter. Anything else is optional and narrows what you see.

## What about async?

The async equivalent of recipe 2 looks like this:

```python
"""Async listener for test_polygon notifications."""

import asyncio
import os
import aviso


async def main() -> None:
    client = aviso.AsyncAvisoClient(base_url=os.environ["AVISO_BASE_URL"], auth=aviso.Env())
    async for notification in client.listen("test_polygon", filter={"polygon": "0,0,1,0,1,1,0,0"}):
        print(f"seq={notification.sequence}")


asyncio.run(main())
```

If your script is just running aviso, the sync version is what you want. See [Async](./async.md) for the situations where the async client actually helps.
