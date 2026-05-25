# aviso Python examples

Runnable scripts that show what the Python API does, one scenario per file. Every script reads `AVISO_BASE_URL` plus either `AVISO_TOKEN` or `AVISO_USERNAME` / `AVISO_PASSWORD` from the environment and connects to whatever aviso-server you point it at. The examples use the `test_polygon` event type because it is widely available on dev servers; if your server has different schemas configured, replace the event type and identifier fields with what `client.schema()` reports.

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
- **triggers/**: the five trigger kinds that you might attach to a watch. Each shows the kwargs path (`triggers=[...]`).
- **resilience/**: state-store resume and exception handling. The two reasons you need patterns on top of listen.
- **async/**: the async client. The basic listener and the multiplex pattern that earns the async surface its keep.
- **advanced/**: the builder pattern (alternative to kwargs), replay-only mode, and the runnable webhook example.

Open the per-directory `README.md` for the list of files and a one-line description of each.

## Shared helpers

`_common.py` carries three helpers used by the example bodies:

- `require_env()` validates `AVISO_BASE_URL` plus credentials and returns the base URL. Missing env vars produce a one-line error rather than a confusing `KeyError`.
- `break_after(iterator, n)` yields the first `n` items from an iterator then stops. Examples use this so they terminate in bounded time.
- `temp_dir()` is a `with`-statement helper that yields a `pathlib.Path` to a fresh temporary directory; trigger side effects (log files, state files) write inside it.
