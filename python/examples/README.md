<!--
SPDX-FileCopyrightText: 2026 European Centre for Medium-Range Weather Forecasts (ECMWF)
SPDX-License-Identifier: Apache-2.0
-->

# pyaviso examples

Runnable scripts that show what the Python API does, one scenario per file. Every script reads `AVISO_BASE_URL` plus either `AVISO_TOKEN` or `AVISO_USERNAME` / `AVISO_PASSWORD` from the environment and connects to whatever aviso-server you point it at. The examples use the `test_polygon` event type, which the local e2e stack in this repo ships pre-configured.

## Install the pyaviso package

The `pyaviso` package must be installed in the active Python environment. From a checkout:

```bash
uv sync --locked --group dev
uv run maturin develop --release --locked
```

See [`python/README.md`](../README.md#install) for the full install instructions.

## Run the examples against the local stack (recommended)

The fastest way to try the examples is the docker-compose stack from `tests/e2e/`. It ships `test_polygon` pre-configured (auth required, write role `producer`), and the `producer-user` account has both read and write permissions.

```bash
# bring the stack up (auth-o-tron + aviso-server + JetStream NATS)
bash tests/e2e/shared/stack.sh up

# point the examples at the local stack
export AVISO_BASE_URL=http://localhost:8000
export AVISO_USERNAME=producer-user
export AVISO_PASSWORD=producer-pass

# run any example
uv run python python/examples/basics/01_publish.py

# when done
bash tests/e2e/shared/stack.sh down
```

See [`tests/e2e/README.md`](../../tests/e2e/README.md) for the two read-only test accounts (`admin-user`, `reader-user`) and the full lifecycle commands.

## Run the examples against your own server

Set the same three environment variables to point at an aviso-server you can reach. The examples assume `test_polygon` is configured on the server; if it is not, see [the quickstart's "What is on your server" section](../../docs/src/python/quickstart.md#what-is-on-your-server) to discover your server's schemas, then substitute your own event type and identifier fields in each example (the shape of every call stays the same).

Each script terminates on its own (most listeners stop after receiving 3 notifications) so you do not have to Ctrl+C anything. The terminate-after-N pattern is for the harness and for users following along; remove the `break_after(...)` call in real long-running code.

## Layout

The directories group examples by purpose, not by API surface:

- **basics/**: the three calls that every user needs first. Publish, listen, schema discovery.
- **triggers/**: four trigger-kind examples (`echo`, `log`, `command`, `webhook`) plus one composition example (`multiple`), all using the kwargs path (`triggers=[...]`). `teams` and `post` are HTTP variants of `webhook` and share its shape; see the [api reference](../../docs/src/python/api-reference.md#triggers) for their full constructors.
- **resilience/**: state-store resume and exception handling. The two reasons you need patterns on top of listen.
- **async/**: the async client. The basic listener and the multiplex pattern that earns the async surface its keep.
- **advanced/**: the builder pattern (alternative to kwargs) and replay-only mode.

Open the per-directory `README.md` for the list of files and a one-line description of each.

## Shared helpers

`_common.py` carries three helpers used by the example bodies:

- `require_env()` validates `AVISO_BASE_URL` plus credentials and returns the base URL. Missing env vars produce a one-line error rather than a confusing `KeyError`.
- `break_after(iterator, n)` yields the first `n` items from an iterator then stops. Examples use this so they terminate in bounded time.
- `temp_dir()` is a `with`-statement helper that yields a `pathlib.Path` to a fresh temporary directory; trigger side effects (log files, state files) write inside it.
