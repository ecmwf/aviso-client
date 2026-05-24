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

```python
import aviso

try:
    response = client.notify(event_type="mars", payload={"location": "..."})
except aviso.AvisoError as e:
    log.warning("aviso publish failed: %s", e)
```

## Catching specific kinds

`HttpError` exposes the server's response in structured form:

```python
try:
    response = client.notify(event_type="mars", payload={"location": "..."})
except aviso.HttpError as e:
    log.warning("server said %d: %s (request_id=%s)", e.status, e.body, e.request_id)
    if e.status == 401:
        raise
```

`HistoryGapError` carries `.reason` plus reason-specific fields:

```python
try:
    for n in client.listen("mars", filter={"class": "od"}):
        ...
except aviso.HistoryGapError as e:
    if e.reason == "replay_limit_reached":
        log.warning("server cap hit; some backfill unavailable, max=%d", e.max_allowed)
    elif e.reason == "sequence_jump":
        log.warning("gap on the wire: expected %d, observed %d", e.expected, e.observed)
```

`TriggerError` carries `.trigger_kind` and `.error_kind` discriminators plus per-kind fields:

```python
try:
    for n in client.listen("mars", filter={"class": "od"}):
        ...
except aviso.TriggerError as e:
    log.warning(
        "trigger %s (%s) failed: status=%s reason=%s",
        e.trigger_kind, e.error_kind, e.status, e.reason,
    )
```

## When errors propagate

- `notify` / `schema` / `schema_for` / admin methods raise on error and return on success. Errors are not auto-retried (except the auth-refresh-on-401 round trip, which retries the original request once with refreshed credentials).
- `listen` iteration raises errors mid-stream: the next `__next__` / `__anext__` call yields the exception, then subsequent calls behave as if the iterator is exhausted. The supervisor has already cancelled by that point.
- A required trigger that fails after all retries terminates the watch with `TriggerError`; the committed cursor stays where it was, so the next process start re-delivers the notification whose trigger failed.

## Catching is not the same as recovering

Some errors are recoverable (`HttpError` with a 5xx is worth a retry; `AuthError` after fixing the token is fine). Others are not (`MalformedEventError` is terminal per the protocol; reconnecting would re-receive the same bad event). The library raises both classes through the same hierarchy; the caller decides whether to retry, alert, or stop.
