# Troubleshooting

Common failure modes and how to fix them.

## `import pyaviso` fails with `ModuleNotFoundError: No module named 'pyaviso._native'`

The compiled extension was not built. From a checkout:

```bash
uv sync --locked --group dev
uv run maturin develop --release --locked
```

The `--group dev` flag is what makes `maturin` available in the environment;
without it `uv run maturin` will not find the executable. `--locked` on both
commands matches the CI workflow and avoids unexpected `Cargo.lock`
modifications during local builds.

If the build itself fails, check that you have Rust installed
(`rustc --version`) and a C compiler on the path.

## `pyaviso.ConfigError: invalid base_url`

Pass a full URL including the scheme:

<!-- not-runnable -->
```python
pyaviso.AvisoClient(base_url="https://aviso.example.org")
```

`localhost`, `aviso.example.org`, and `//aviso.example.org` are all rejected
because the underlying URL parser cannot read them as absolute.

## `pyaviso.HttpError: 401`

The configured auth source did not produce credentials the server accepts. Check
that:

- `Bearer` was constructed with a token the server's auth backend recognises.
- `Basic` was constructed with the right username and password.
- `Env()` is reading the env vars you expect (`AVISO_TOKEN`, `AVISO_USERNAME`,
  `AVISO_PASSWORD`).
- `ConfigFile(...)` points at a file with exactly one of `bearer:` or `basic:`
  at the top level.

## `pyaviso.TransportError`

Network failure before the response begins. The error message names the cause
(DNS, TCP, TLS). For self-signed certificates in dev, use
`pyaviso.AvisoClient(base_url="...", danger_accept_invalid_certs=True)` and
accept the loud warning that comes with it.

## `pyaviso.HistoryGapError`

A gap was detected in the watch stream. Two reasons:

- `reason == "replay_limit_reached"`: the server's
  `notification_replay_limit_reached` payload says some of the requested
  backfill is older than its retention. `.max_allowed` tells you how many
  notifications can be replayed at most.
- `reason == "sequence_jump"`: the wire delivered a non-consecutive sequence.
  `.expected` and `.observed` name the boundary.

A gap is terminal: continuing past it would silently violate at-least-once. The
client raises and exits the iterator. Decide what the right recovery is (for
example, restart from the live edge with `from_=None`).

## `pyaviso.TriggerError: command failed`

A required command trigger exited non-zero or timed out. The exception carries:

- `.exit_code`: the child's exit code (`-1` for signal-terminated).
- `.stderr_tail`: the last 4 KiB of the child's stderr.

If the failure is transient, raise the trigger's `retries=` count or set
`required=False` so the watch continues past a failed dispatch.

## `Ctrl+C` does not stop a listening loop

The sync iterator polls the channel every 100 ms and checks for pending signals
between polls. If it takes longer than that to respond, you may have a Python
operation in the loop body that does not yield. Move heavy work into a
background thread or use the async client.

## `pyaviso.StateStoreError` on first run

`JsonFileStore` does not create parent directories. Create them first:

```python
"""Construct the state file's parent directory before passing it to the client."""

import os
import pathlib
import pyaviso

path = pathlib.Path("~/.config/aviso/state.json").expanduser()
path.parent.mkdir(parents=True, exist_ok=True)

client = pyaviso.AvisoClient(
    base_url=os.environ["AVISO_BASE_URL"],
    auth=pyaviso.Env(),
    state_store=pyaviso.JsonFileStore(path),
)

print(f"using state file: {path}")
```

## Mixing sync and async clients

Do not call sync methods on `AvisoClient` from inside an asyncio event loop. The
sync surface drives the underlying tokio runtime with `block_on`, which blocks
the asyncio thread until the call returns. Other coroutines stop making progress
until then. Use `AsyncAvisoClient` from inside async code, or push the sync
client onto a thread:

<!-- not-runnable -->
```python
import asyncio
result = await asyncio.to_thread(
    client.notify,
    event_type="test_polygon",
    identifier={"polygon": "0,0,1,0,1,1,0,0", "date": "20260601", "time": "1200"},
    payload={"location": "s3://example/data.grib"},
)
```

See [the Async page](./async.md) for the situations where the async client
actually helps.

## Wheels on PyPI

Not available today. Install from source with `uv run maturin develop`. If and
when the wheel matrix lands, `pip install pyaviso` becomes the simpler path.
