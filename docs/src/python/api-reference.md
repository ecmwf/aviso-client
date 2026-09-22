<!--
SPDX-FileCopyrightText: 2026 European Centre for Medium-Range Weather Forecasts (ECMWF)
SPDX-License-Identifier: Apache-2.0
-->

# API reference

Use the [quickstart](./quickstart.md) for your first script. This reference is
for looking up signatures and return values. Import the public `pyaviso` module.

For motivation and when-to-use-this guidance on individual surfaces, see the
narrative pages. The async client is documented on [the Async page](./async.md);
HTTP methods return awaitables on the async client. `listen()` returns an async
iterator directly, without `await`.

## Clients

<!-- not-runnable -->
```python
class pyaviso.AvisoClient(
    *,
    base_url: str,
    auth: AuthProvider | None = None,
    timeout: float | None = None,
    user_agent: str | None = None,
    state_store: StateStore | None = None,
    heartbeat_interval: float | None = None,
    danger_accept_invalid_certs: bool = False,
    flush_cursor_on_exit: bool = False,
)
```

### Discover schemas

- `schema() -> SchemaCatalog`
- `schema_for(event_type) -> SchemaResponse`

### Receive notifications

- `listen(event_type=None, *, filter=None, start_from=None, mode=None, triggers=None, request=None) -> NotificationIterator`
  (mode defaults to `"watch"` when not specified; `triggers` is a
  `Sequence[Trigger]`; the returned iterator is also a context manager via
  `with` and supports `iterator.close()` for explicit teardown)

`start_from` is `int | str | None`. Integers are exclusive sequence positions;
`0` requests retained history after zero. A UTC string such as
`"2026-06-01T00:00:00Z"` selects publication time. `None` uses saved state if
available, otherwise the live edge. An explicit start overrides saved state.
The value's type selects the meaning, so sequence numbers must be unquoted;
see [Start from a specific position](./listen.md#start-from-a-specific-position).
Use `mode="replay_only"` with a start position to end at the replay boundary.
See [State and resume](./state-and-resume.md) for checkpoint limits.

### Publish notifications (providers)

- `notify(*, event_type, identifier=None, payload=None) -> NotifyResponse`
- `notify_many(notifications, *, concurrency=0) -> list[NotifyResult]`

`notify_many` takes a sequence of notification dicts and returns results in
input order. `concurrency=0` limits it to 16 in-flight requests. A batch is not
atomic; inspect each result. See [Publishing](./publish.md).

`identifier` is a mapping from strings to JSON-compatible Python values. The
same applies to each notification passed to `notify_many`. Structured values
such as point-cloud lists are sent as JSON arrays. Cyclic containers are not
valid JSON and raise `TypeError`. Identifier input deeper than 100 nested
containers raises `ValueError` before conversion.

Supply every identifier field defined by the server when publishing, even
fields optional in filters. `payload=None` omits the payload; the schema decides
whether a payload is required.

### Admin operations (operators)

- `wipe_stream(stream_name) -> None`
- `wipe_all() -> None`
- `delete_notification(notification_id) -> None`

These remove retained data and require appropriate permission. A publish
response's request ID is not a notification ID.

### Constructor options and lifecycle

`base_url` is required and includes the HTTP or HTTPS scheme. `auth=None`
searches the environment and the credential files; see
[Authentication](./auth.md). Pass `auth=pyaviso.Anonymous()` to send no
credentials. `state_store=None` does not persist checkpoints.
`flush_cursor_on_exit=False` leaves the final pending cursor unflushed;
enabling it does not acknowledge completed work.
See [shutdown behavior](./state-and-resume.md#flush-on-exit).

`timeout` and `heartbeat_interval` are seconds. `timeout=None` imposes no
request timeout. `heartbeat_interval=None` uses 30 seconds as the expected
server heartbeat interval; it does not set the server's cadence.
`user_agent=None` uses `aviso/<crate-version>`.
`danger_accept_invalid_certs=False` keeps certificate validation enabled.

Use `with client.listen(...)` to close a synchronous listener. The synchronous
client's own `__enter__` / `__exit__` do not close its listeners. The async
client is not an async context manager; use `async with` on its iterator.

`pyaviso.AsyncAvisoClient` is the same shape. `notify` / `schema` /
`notify_many` / `schema_for` / `wipe_*` / `delete_notification` return
awaitables; `listen` returns an `AsyncNotificationIterator`.

## Value types

`pyaviso.Notification(event_type, sequence, identifier, payload, cloudevent=None)`

Properties: `event_type`, `sequence`, `identifier`, `payload`, `cloudevent`.
Method: `as_dict()`. Unhashable (the payload may be a dict).

`str(notification)` formats the original `cloudevent` as indented JSON. For a
manually constructed notification without `cloudevent`, it formats `event_type`,
`sequence`, `identifier`, and `payload` instead. `as_dict()` returns the client
convenience fields, including `cloudevent` when present. `repr(notification)`
remains a compact debugging summary.
Identifier values retain the JSON shape emitted by the server.

`pyaviso.NotifyResponse(status, request_id, processed_at)`

Properties: `status`, `request_id`, `processed_at`. Method: `as_dict()`.

`pyaviso.NotifyResult` (returned by `notify_many`)

Properties: zero-based `index`, boolean `ok`, `response` (`NotifyResponse` on
success, otherwise `None`) and `error` (exception on failure, otherwise `None`).

`pyaviso.SchemaCatalog`

Properties: `status`, `event_types`, `total_schemas`, `schema`. Method:
`as_dict()`. The `schema` property is a dict keyed by event type, whose values
match the per-event-type schema below.

`pyaviso.SchemaResponse`

Properties: `status`, `event_type`, `schema`. Method: `as_dict()`. The `schema`
is a dict with two keys: `payload` (a `{"required": bool}` shape) and
`identifier` (a dict keyed by identifier field name, each value naming the
validator type, the `required` flag (which gates whether a filter or watch call
must include the field, not whether a notify call must), and any type-specific
metadata).

## Watch shape

`pyaviso.WatchRequest`

Constructors:

- `WatchRequest.watch(event_type)`
- `WatchRequest.watch_from(event_type, start_from)`
- `WatchRequest.replay_only(event_type, start_from)`

Builders: `.with_filter(dict)`, `.with_triggers(list)`. Properties:
`event_type`, `mode`.

`pyaviso.WatchMode` is a `str + Enum` with members `WATCH = "watch"` and
`REPLAY_ONLY = "replay_only"`.

`pyaviso.NotificationIterator` and `pyaviso.AsyncNotificationIterator`: returned
by `listen`. The sync one implements `__iter__` / `__next__` / `close`; the
async one implements `__aiter__` / `__anext__` / `aclose`. Both are iterator
context managers: `with` closes the sync iterator; `async with` awaits async
closure. A loop `break` alone does not close an iterator you still hold.

## Triggers

`pyaviso.Trigger` with class-method constructors:

- `Trigger.echo(*, retries=0, required=True, label=None)`
- `Trigger.log(path, *, retries=0, required=True)`
- `Trigger.command(cmd, *, env=None, working_dir=None, retries=0, required=True, timeout=None, fail_fast=True)`
  (Unix only)
- `Trigger.webhook(url, *, method=None, headers=None, body_template=None, retries=0, required=True, timeout=30.0, fail_fast=True)`
- `Trigger.teams(url, *, retries=0, required=True, timeout=30.0, fail_fast=True)`
- `Trigger.post(url, *, retries=0, required=True, timeout=30.0, fail_fast=True)`

Chainable setters: `.retries(n)`, `.required(on)`, `.timeout(seconds)`,
`.fail_fast(on)`, `.label(name)`.

Setters return new values. Webhook `method=None` means POST. Timeout and
fail-fast setters affect only command and HTTP triggers; label affects only
echo. Echo and log output the smaller notification view, while post forwards
the original CloudEvent. See [Triggers](./triggers.md).

`pyaviso.HttpMethod` is a `str + Enum` with members `POST`, `GET`, `PUT`,
`PATCH`, `DELETE`.

## Auth providers

`pyaviso.Bearer(token)`, `pyaviso.Basic(username, password="")`,
`pyaviso.Env()`, `pyaviso.ConfigFile(path)`, `pyaviso.Chain(*providers)`.

Type alias: `pyaviso.AuthProvider = Bearer | Basic | Env | ConfigFile | Chain`.

## State stores

`pyaviso.MemoryStore()`, `pyaviso.JsonFileStore(path)`.

Type alias: `pyaviso.StateStore = MemoryStore | JsonFileStore`.

## Exceptions

Base: `pyaviso.AvisoError`.

Subclasses:

- `pyaviso.TransportError`
- `pyaviso.HttpError` (attrs: `status`, `body`, `request_id`)
- `pyaviso.AuthError`
- `pyaviso.DecodeError`
- `pyaviso.MalformedEventError`
- `pyaviso.HistoryGapError` (attrs: `reason`, `max_allowed`, `expected`,
  `observed`)
- `pyaviso.StreamProtocolError` (attrs: `message`, `request_id`)
- `pyaviso.ConfigError`
- `pyaviso.StateStoreError`
- `pyaviso.TriggerError` (attrs: `trigger_kind`, `error_kind`, `path`,
  `exit_code`, `stderr_tail`, `status`, `body_tail`, `reason`,
  `timeout_seconds`, `context`, `field`, `template_kind`)

## Version

`pyaviso.__version__` is the Python distribution version. `pyaviso.VERSION`
is the Rust crate version. They are pinned to the same value via maturin's
`dynamic = ["version"]`.
