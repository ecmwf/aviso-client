# Python

`pyaviso` is a Python library for publishing to and listening on an
`aviso-server`. The package wraps the Rust core through PyO3 bindings, so the
same delivery guarantees, reconnect behaviour, and trigger surface that back the
CLI are available from Python.

The default client is synchronous. You write straight-line Python: construct a
client, call `notify`, iterate `listen`. No event loop, no `async def`, no
`await`. If your script is built around aviso, this is the surface you want.

```python
import os
import pyaviso

client = pyaviso.AvisoClient(base_url=os.environ["AVISO_BASE_URL"], auth=pyaviso.Env())

for notification in client.listen("test_polygon", filter={"polygon": "0,0,1,0,1,1,0,0"}):
    print(notification.sequence, notification.payload)
```

## What you get

- `notify`, `schema`, `schema_for`, `wipe_stream`, `wipe_all`,
  `delete_notification` on the client.
- `listen`, returning an iterator of `Notification` instances. Sync iteration
  with `for`.
- A real exception hierarchy rooted at `pyaviso.AvisoError` with structured
  attributes per error kind.
- Auth providers: `Bearer`, `Basic`, `Env`, `ConfigFile`, `Chain`.
- State stores: `MemoryStore`, `JsonFileStore`.
- Triggers: `Trigger.echo`, `.log`, `.command`, `.webhook`, `.teams`, `.post`
  with chainable setters and keyword-argument constructors.
- Value types: `Notification`, `NotifyResponse`, `SchemaCatalog`,
  `SchemaResponse`, each with `as_dict()` for easy JSON / pandas / dataclass
  interop.

## What about async?

An `AsyncAvisoClient` ships in the same package. It is the same shape as
`AvisoClient` with `await` and `async for` instead of `for`. You need it in
three situations: you are inside an existing asyncio app (FastAPI, aiohttp), you
want to drain several streams concurrently from one process, or you want to fan
out publishes with `asyncio.gather`. Outside those, sync is shorter, simpler,
and just as fast for one stream.

The full guide is on [the Async page](./async.md).

## Where to go next

- [Install](./install.md) walks through the source-install workflow that works
  today.
- [Quickstart](./quickstart.md) is three end-to-end scripts you can paste and
  run.
- [Publishing](./publish.md) and [Listening](./listen.md) go deeper on the two
  main verbs.
- [Auth](./auth.md), [State and resume](./state-and-resume.md), and
  [Error handling](./error-handling.md) cover the surrounding concerns.
- [Triggers](./triggers.md) shows the six declarative side-effect kinds.
- [Builder pattern](./builder-pattern.md) is the fluent way to build reusable
  triggers and watch requests.
- [Async](./async.md) is the page for the second client class.
- [API reference](./api-reference.md) lists every public symbol.
- [Troubleshooting](./troubleshooting.md) is the page to read when something is
  wrong.
