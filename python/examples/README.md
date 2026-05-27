# aviso Python examples

Runnable scripts that show what the Python API does, one scenario per file. Every script reads `AVISO_BASE_URL` plus either `AVISO_TOKEN` or `AVISO_USERNAME` / `AVISO_PASSWORD` from the environment and connects to whatever aviso-server you point it at. The examples use the `test_polygon` event type because it is widely available on dev servers; if your server has different schemas configured, replace the event type and identifier fields with what `client.schema()` reports.

## Prerequisites

The `aviso` package must be installed in the active Python environment. From a checkout:

```bash
uv sync --locked --group dev
uv run maturin develop --release --locked
```

See [`python/README.md`](../README.md#install) for the full install instructions.

The examples use the `test_polygon` event type because it is widely available on dev servers. If `client.schema().event_types` against your server does not include it, you have two options:

- **If you have access to the aviso-server config**: paste the `test_polygon` snippet from the [quickstart's "What is on your server" section](../../docs/src/python/quickstart.md#what-is-on-your-server) into your server's `notification_schema:` and restart. The same definition is in [`tests/e2e/aviso-server.config.yaml`](../../tests/e2e/aviso-server.config.yaml).
- **If you only use the server (not run it)**: substitute your own event type and identifier fields in each example. The shape of every call stays the same; only the event-type string and identifier keys change.

## How to run

Set two environment variables and run any script:

```bash
export AVISO_BASE_URL=https://aviso.example.org
export AVISO_USERNAME=alice
export AVISO_PASSWORD=wonderland

python python/examples/basics/01_publish.py
```

Each script terminates on its own (most listeners stop after receiving 3 notifications) so you do not have to Ctrl+C anything. The terminate-after-N pattern is for the harness and for users following along; remove the `break_after(...)` call in real long-running code.

## Layout

The directories group examples by purpose, not by API surface:

- **basics/**: the three calls that every user needs first. Publish, listen, schema discovery.
- **triggers/**: four trigger-kind examples (`echo`, `log`, `command`, `webhook`) plus one composition example (`multiple`), all using the kwargs path (`triggers=[...]`). `teams` and `post` are HTTP variants of `webhook` and share its shape; see the [api reference](../../docs/src/python/api-reference.md#triggers) for their full constructors.
- **resilience/**: state-store resume and exception handling. The two reasons you need patterns on top of listen.
- **async/**: the async client. The basic listener and the multiplex pattern that earns the async surface its keep.
- **advanced/**: the builder pattern (alternative to kwargs), replay-only mode, and the runnable webhook example.

Open the per-directory `README.md` for the list of files and a one-line description of each.

## Shared helpers

`_common.py` carries three helpers used by the example bodies:

- `require_env()` validates `AVISO_BASE_URL` plus credentials and returns the base URL. Missing env vars produce a one-line error rather than a confusing `KeyError`.
- `break_after(iterator, n)` yields the first `n` items from an iterator then stops. Examples use this so they terminate in bounded time.
- `temp_dir()` is a `with`-statement helper that yields a `pathlib.Path` to a fresh temporary directory; trigger side effects (log files, state files) write inside it.
