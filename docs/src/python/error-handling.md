# Error handling

Every exception the library raises subclasses `aviso.AvisoError`. Catch that to handle anything from the library; catch a specific class for fine-grained dispatch.

## Hierarchy

```text
AvisoError
├── TransportError
├── HttpError
├── AuthError
├── DecodeError
├── MalformedEventError
├── HistoryGapError
├── StreamProtocolError
├── ConfigError
├── StateStoreError
└── TriggerError
```

## Catching everything

The simplest pattern catches `AvisoError` for any library-originated failure:

```python
"""Publish and log any library-side failure."""

import logging
import os
import aviso

logging.basicConfig(level=logging.WARNING)
log = logging.getLogger("publish")

client = aviso.AvisoClient(base_url=os.environ["AVISO_BASE_URL"], auth=aviso.Env())

try:
    response = client.notify(
        event_type="test_polygon",
        identifier={
            "polygon": "0,0,1,0,1,1,0,0",
            "date": "20260601",
            "time": "1200",
        },
        payload={"location": "s3://example/data.grib"},
    )
    print(f"ok: {response.request_id}")
except aviso.AvisoError as e:
    log.warning("aviso publish failed: %s", e)
```

## `HttpError` exposes the server's response

`HttpError` carries `.status`, `.body`, and `.request_id`. The body is whatever the server sent; for aviso-server it is a JSON object with a `code`, a `details` message, and the same `request_id` for support correlation.

```python
"""Construct an invalid notify call on purpose; inspect the HttpError."""

import os
import aviso

client = aviso.AvisoClient(base_url=os.environ["AVISO_BASE_URL"], auth=aviso.Env())

try:
    client.notify(
        event_type="test_polygon",
        identifier={"polygon": "not-a-polygon", "date": "20260601", "time": "1200"},
        payload={"location": "s3://example/data.grib"},
    )
except aviso.HttpError as e:
    print(f"status={e.status}")
    print(f"request_id={e.request_id}")
    print(f"body={e.body[:200]}")
```

Expected output (one block per run; the UUID changes every run, the rest is fixed for this specific bad input):

```text
status=400
request_id=3dc3a144-e33c-465f-bfb8-bfcf01044e2f
body={"code":"INVALID_NOTIFICATION_REQUEST","details":"field 'polygon' must be a valid polygon: polygon coordinates must be in lat,lon pairs (got an odd number of values)",...}
```

The `request_id` is the value you would quote to operations or in a support ticket to find the request in server logs.

## `HistoryGapError` carries a reason

`HistoryGapError` is raised mid-stream when the supervisor detects a gap that would violate at-least-once. It carries a `.reason` discriminator plus reason-specific fields.

<!-- not-runnable -->
```python
import os
import aviso

client = aviso.AvisoClient(base_url=os.environ["AVISO_BASE_URL"], auth=aviso.Env())

try:
    for n in client.listen("test_polygon", filter={"polygon": "0,0,1,0,1,1,0,0"}):
        ...
except aviso.HistoryGapError as e:
    if e.reason == "replay_limit_reached":
        print(f"server cap hit; max replayable = {e.max_allowed}")
    elif e.reason == "sequence_jump":
        print(f"wire gap: expected {e.expected}, observed {e.observed}")
```

A gap is terminal: the iterator stops and the supervisor exits. Decide what the right recovery is for your case. Common moves are restart from the live edge with `from_=None`, or restart from a specific known-good sequence with `from_=<n>`.

## `TriggerError` carries a kind and a sub-kind

When a required trigger fails after all its retries, the watch terminates with `TriggerError`. The exception carries `.trigger_kind` (which trigger), `.error_kind` (what went wrong), and a set of per-kind fields:

<!-- not-runnable -->
```python
import os
import aviso

client = aviso.AvisoClient(base_url=os.environ["AVISO_BASE_URL"], auth=aviso.Env())

request = (
    aviso.WatchRequest.watch("test_polygon")
    .with_filter({"polygon": "0,0,1,0,1,1,0,0"})
    .with_triggers([aviso.Trigger.command("./process.sh {{ notification.sequence }}")])
)

try:
    for n in client.listen(request=request):
        ...
except aviso.TriggerError as e:
    print(f"trigger={e.trigger_kind} kind={e.error_kind}")
    if e.error_kind == "command":
        print(f"  exit_code={e.exit_code} stderr_tail={e.stderr_tail!r}")
    elif e.error_kind == "webhook":
        print(f"  status={e.status} body_tail={e.body_tail!r}")
    elif e.error_kind == "timeout":
        print(f"  timeout_seconds={e.timeout_seconds}")
    elif e.error_kind == "template":
        print(f"  context={e.context!r} field={e.field!r} template_kind={e.template_kind!r}")
```

`trigger_kind` is one of `echo`, `log`, `command`, `webhook`, `teams`, `post`, or `unknown`. `error_kind` is one of `io`, `encode`, `command`, `timeout`, `webhook`, `webhook_build`, `template`, or `unknown`. Per-kind fields are populated only when relevant; the rest are `None`.

## When errors propagate

- `notify`, `schema`, `schema_for`, and the admin methods raise on error and return on success. Errors are not auto-retried (except the auth-refresh-on-401 round trip, which retries the original request once with refreshed credentials).
- `listen` iteration raises errors mid-stream. The next `__next__` call yields the exception; subsequent calls behave as if the iterator is exhausted. The supervisor has already cancelled by that point.
- A required trigger that fails after all retries terminates the watch with `TriggerError`. The committed cursor stays where it was, so the next process start re-delivers the notification whose trigger failed.

## Catching is not the same as recovering

Some errors are recoverable. An `HttpError` with a 5xx status is worth a retry. An `AuthError` after fixing the token is fine. Others are terminal: a `MalformedEventError` is fatal per the protocol because reconnecting would re-receive the same bad event. The library raises both classes through the same hierarchy; the caller decides whether to retry, alert, or stop.

## With `AsyncAvisoClient`

The async client raises the same exceptions through the same hierarchy. The difference is calling style: `await` on a method or `async for` over `listen` raises the same way `client.notify(...)` raises in the sync surface.
