# Python

`aviso` is a Python library that publishes to and listens on an `aviso-server`. The package wraps the Rust core via PyO3 bindings, so the same delivery guarantees, reconnect behaviour, and trigger surface that back the CLI are available from Python.

Two client classes ship in the package:

- `aviso.AvisoClient` is synchronous. Methods block the calling thread; iteration uses a plain `for` loop. Reach for it in scripts and short-lived processes where you do not already run an asyncio event loop.
- `aviso.AsyncAvisoClient` is asynchronous. Methods return awaitables; iteration uses `async for`. Reach for it from inside an asyncio application or when you need to interleave aviso work with other async I/O.

Both classes share the same constructor shape, the same auth and state-store wiring, and the same exception hierarchy. The choice is about call style, not capability.

## What you get

- `notify`, `schema`, `schema_for`, `wipe_stream`, `wipe_all`, `delete_notification` on every client.
- `listen` on every client, returning an iterator (sync) or async iterator (async) of `Notification` instances.
- A real exception hierarchy rooted at `aviso.AvisoError` with structured attributes per error kind.
- Auth providers: `Bearer`, `Basic`, `Env`, `ConfigFile`, `Chain`.
- State stores: `MemoryStore`, `JsonFileStore`.
- Triggers: `Trigger.echo`, `.log`, `.command`, `.webhook`, `.teams`, `.post` with chainable setters and keyword-argument constructors.
- Pythonic value types: `Notification`, `NotifyResponse`, `SchemaCatalog`, `SchemaResponse`, all with `as_dict()` for easy JSON / pandas / dataclass interop.

## Where to go next

- [Install](./install.md) describes the source install workflow that works today.
- [Quickstart](./quickstart.md) walks the three most common recipes end to end.
- [Listening](./listen.md) and [Publishing](./publish.md) go deeper on the two main verbs.
- [API reference](./api-reference.md) lists every public symbol.
