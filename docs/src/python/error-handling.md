<!--
SPDX-FileCopyrightText: 2026 European Centre for Medium-Range Weather Forecasts (ECMWF)
SPDX-License-Identifier: Apache-2.0
-->

# Error handling

Read the exception message first. A missing credential needs a setup change;
a rejected filter needs a schema check. Repeating the same request will not
necessarily fix either problem.

Client errors inherit from `pyaviso.AvisoError`. Python input errors can also
raise `TypeError` or `ValueError`, and a missing environment variable accessed
through `os.environ[...]` raises `KeyError`. `AvisoError` does not catch these
or errors in your own analysis.

## Catching everything

Use the [quickstart setup](./quickstart.md#set-the-environment), including
`AVISO_BASE_URL` and credentials for `pyaviso.Env()`. For an anonymous server,
pass `auth=pyaviso.Anonymous()` instead.

This listener uses the
[small `mars` schema](./quickstart.md#what-is-on-your-server).
It has a required `class` choice (`od` or `rd`) and optional whole-number `step`
filter.
Providers must supply both identifiers; the payload is optional. Listening
requires receiving permission, not publishing permission.

Save this as `listen_errors.py` and run `python listen_errors.py`:

```python
import os

import pyaviso

try:
    client = pyaviso.AvisoClient(
        base_url=os.environ["AVISO_BASE_URL"], auth=pyaviso.Env()
    )
    with client.listen("mars", filter={"class": "od"}) as notifications:
        for notification in notifications:
            print(notification)
except pyaviso.AvisoError as error:
    print("Listening failed:", error)
    raise
except KeyboardInterrupt:
    print("Stopped listening")
```

It prints matching notifications as indented CloudEvent JSON until you press
Ctrl+C. Setup is inside `try` because `Env()` can fail before listening starts.
The `with` block closes the iterator even if your loop raises an exception.
The error branch reports the failure and reraises it, so a failed job does not
look like a successful run.

## `HttpError` exposes the server's response

`HttpError` carries the integer `status`, string `body` and optional
`request_id`. Keep the request ID when asking your operator to find the request
in server logs. The response body explains what the server rejected.

For providers, here is an intentionally invalid publish. Save it as
`bad_publish.py` and run `python bad_publish.py` with publishing credentials and
the same environment setup:

```python
import json
import os

import pyaviso

client = pyaviso.AvisoClient(
    base_url=os.environ["AVISO_BASE_URL"], auth=pyaviso.Env()
)
try:
    client.notify(event_type="mars", identifier={"class": "od"})
except pyaviso.HttpError as error:
    print("status:", error.status)
    print(json.dumps(json.loads(error.body), indent=2, sort_keys=True))
```

Captured output with the small `mars` schema (your request ID will differ):

```text
status: 400
{
  "code": "INVALID_NOTIFICATION_REQUEST",
  "details": "Required field 'step' missing for notify operation",
  "error": "Invalid Notification Request",
  "message": "Required field 'step' missing for notify operation",
  "request_id": "daf5f6d6-6f40-42db-a1e5-8b22df279b07"
}
```

The server rejects this with HTTP 400 because `step` is missing.
It is optional in listener filters, but required when publishing. This example
decodes the Aviso server's JSON error body; a proxy or another server can return
plain text, so general error handlers should not assume every body is JSON.

## When errors propagate

- Setup can fail when constructing a provider, client, request or state store.
- HTTP methods such as `notify` and `schema` raise when the request fails.
  `notify_many` returns per-notification results; inspect every result. Invalid
  batch input can raise before any request is sent.
- Listening can fail when opening the iterator or during iteration. After a
  terminal stream error is delivered, subsequent iteration is exhausted.
- A required trigger failure stops the listener before that notification
  reaches your loop. Earlier trigger effects are not undone.

Connection losses and retryable server responses normally cause listeners to
reconnect. One-shot publishes are not automatically retried after transport
failure: the server may already have stored the notification. An auth provider
is asked to refresh after HTTP 401, followed by one retry if refresh succeeds.
If authentication is still rejected while opening a watch, the listener raises
`AuthError`. A one-shot HTTP request instead exposes the final 401 as
`HttpError`.
See [Authentication](./auth.md#refresh-on-401).

## `HistoryGapError` carries a reason

A history gap ends the iterator. Inspect `error.reason`:

| Reason | Meaning | Useful fields |
|---|---|---|
| `replay_limit_reached` | The server capped the requested replay | `max_allowed` |
| `sequence_jump` | The protocol reported an unexpected sequence boundary | `expected`, `observed` |

Do not treat a failed replay as complete. Check server retention and replay
limits with the operator, then choose a starting point appropriate to your
work. Starting live skips historical work. Also, `start_from=None` uses an
existing saved cursor when one is configured; it does not override it. See
[choosing a starting position](./state-and-resume.md#choose-a-starting-position).

## `TriggerError` carries a kind and a sub-kind

`trigger_kind` identifies the action: `echo`, `log`, `command`, `webhook`,
`teams`, `post` or `unknown`. `error_kind` describes the failure:

| Error kind | Useful fields |
|---|---|
| `command` | `exit_code`, `stderr_tail` |
| `webhook` | `status`, `body_tail` |
| `timeout` | `timeout_seconds` |
| `template` | `context`, `field`, `template_kind` |
| `io` | `path` for a log trigger; read the exception message |
| `webhook_build` | `reason` |
| `encode`, `unknown` | Read the exception message |

Fields that do not apply are `None`. Required triggers stop the watch when their
retry policy is exhausted or fail-fast applies. Optional triggers warn and let
processing continue. Only make an action optional when continuing without it is
acceptable. See [trigger retry settings](./triggers.md#tunables).

The failing notification does not advance the pending cursor. An earlier
notification may already have been checkpointed. Restarting does not guarantee
recovery of unfinished work; see [State and resume](./state-and-resume.md).

## Catching is not the same as recovering

Before retrying a publish, determine whether it may already have succeeded.
For a malformed stream event or protocol error, reconnecting to the same input
may reproduce the failure. Preserve the error and ask the operator to
investigate instead of silently moving past it.

Closing a listener cancels and waits for its background task. It does not
certify that your analysis finished. Ctrl+C handling while waiting for input
does not set a deadline for interrupting Python work or waiting for cleanup.

## Hierarchy

All of these inherit directly from `AvisoError`:

| Exception | What to check |
|---|---|
| `AuthError` | Credential source or watch authentication rejected after refresh |
| `ConfigError` | Client settings, auth file or request options |
| `HttpError` | Server status and response body |
| `TransportError` | Connection or response-transfer failure |
| `DecodeError` | Unexpected response format |
| `MalformedEventError` | Invalid CloudEvent identity |
| `HistoryGapError` | Replay limit or sequence boundary |
| `StreamProtocolError` | Fatal streaming protocol condition |
| `StateStoreError` | Local state path, permissions or contents |
| `TriggerError` | Failed required action |

## With `AsyncAvisoClient`

The same exceptions can arise from an awaited method or `async for` iteration.
Use `async with` on the iterator to await `aclose()` on exit. With
`asyncio.run()`, catch `KeyboardInterrupt` outside the run call, as in the
[async listener](./async.md#a-complete-async-listener). A `TaskGroup` can wrap
task failures in an exception group.
