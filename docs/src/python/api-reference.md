# API reference

Every public symbol the package exports. The Python wrapper module is `aviso`; the compiled extension is `aviso._native` but users never import it directly.

## Clients

```python
class aviso.AvisoClient(
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

Methods:

- `notify(*, event_type, identifier=None, payload=None) -> NotifyResponse`
- `schema() -> SchemaCatalog`
- `schema_for(event_type) -> SchemaResponse`
- `wipe_stream(stream_name) -> None`
- `wipe_all() -> None`
- `delete_notification(notification_id) -> None`
- `listen(event_type=None, *, filter=None, from_=None, mode="watch", request=None) -> NotificationIterator`
- `__enter__` / `__exit__` for `with` blocks.

`aviso.AsyncAvisoClient` is the same shape, with `notify` / `schema` / `schema_for` / `wipe_*` / `delete_notification` returning awaitables and `listen` returning an `AsyncNotificationIterator`.

## Value types

`aviso.Notification(event_type, sequence, identifier, payload, cloudevent=None)`

Properties: `event_type`, `sequence`, `identifier`, `payload`, `cloudevent`. Method: `as_dict()`. Unhashable (the payload may be a dict).

`aviso.NotifyResponse(status, request_id, processed_at)`

Properties: `status`, `request_id`, `processed_at`. Method: `as_dict()`.

`aviso.SchemaCatalog` and `aviso.SchemaResponse`: properties and `as_dict()`.

## Watch shape

`aviso.WatchRequest`

Constructors:

- `WatchRequest.watch(event_type)`
- `WatchRequest.watch_from(event_type, from_)`
- `WatchRequest.replay_only(event_type, from_)`

Builders: `.with_filter(dict)`, `.with_triggers(list)`. Properties: `event_type`, `mode`.

`aviso.WatchMode` is a `str + Enum` with members `WATCH = "watch"` and `REPLAY_ONLY = "replay_only"`.

`aviso.NotificationIterator` and `aviso.AsyncNotificationIterator`: returned by `listen`. The sync one implements `__iter__` / `__next__` / `close`; the async one implements `__aiter__` / `__anext__` / `aclose`.

## Triggers

`aviso.Trigger` with class-method constructors:

- `Trigger.echo(*, retries=0, required=True, label=None)`
- `Trigger.log(path, *, retries=0, required=True)`
- `Trigger.command(cmd, *, env=None, working_dir=None, retries=0, required=True, timeout=None, fail_fast=True)` (Unix only)
- `Trigger.webhook(url, *, method=None, headers=None, body_template=None, retries=0, required=True, timeout=30.0, fail_fast=True)`
- `Trigger.teams(url, *, retries=0, required=True, timeout=30.0, fail_fast=True)`
- `Trigger.post(url, *, retries=0, required=True, timeout=30.0, fail_fast=True)`

Chainable setters: `.retries(n)`, `.required(on)`, `.timeout(seconds)`, `.fail_fast(on)`, `.label(name)`.

`aviso.HttpMethod` is a `str + Enum` with members `POST`, `GET`, `PUT`, `PATCH`, `DELETE`.

## Auth providers

`aviso.Bearer(token)`, `aviso.Basic(username, password="")`, `aviso.Env()`, `aviso.ConfigFile(path)`, `aviso.Chain(*providers)`.

Type alias: `aviso.AuthProvider = Bearer | Basic | Env | ConfigFile | Chain`.

## State stores

`aviso.MemoryStore()`, `aviso.JsonFileStore(path)`.

Type alias: `aviso.StateStore = MemoryStore | JsonFileStore`.

## Exceptions

Base: `aviso.AvisoError`.

Subclasses:

- `aviso.TransportError`
- `aviso.HttpError` (attrs: `status`, `body`, `request_id`)
- `aviso.AuthError`
- `aviso.DecodeError`
- `aviso.MalformedEventError`
- `aviso.HistoryGapError` (attrs: `reason`, `max_allowed`, `expected`, `observed`)
- `aviso.StreamProtocolError` (attrs: `message`, `request_id`)
- `aviso.ConfigError`
- `aviso.StateStoreError`
- `aviso.TriggerError` (attrs: `trigger_kind`, `error_kind`, `path`, `exit_code`, `stderr_tail`, `status`, `body_tail`, `reason`, `timeout_seconds`, `context`, `field`, `template_kind`)

## Version

`aviso.__version__` is the Python distribution version. `aviso.VERSION` is the Rust crate version. They are pinned to the same value via maturin's `dynamic = ["version"]`.
