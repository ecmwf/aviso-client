# Troubleshooting

Common failure modes and how to fix them.

## `import aviso` fails with `ModuleNotFoundError: No module named 'aviso._native'`

The compiled extension was not built. From a checkout:

```bash
uv run maturin develop --release
```

If the build itself fails, check that you have Rust installed (`rustc --version`) and a C compiler on the path.

## `aviso.ConfigError: invalid base_url`

Pass a full URL including the scheme:

```python
aviso.AvisoClient(base_url="https://aviso.example.org")
```

`localhost`, `aviso.example.org`, and `//aviso.example.org` are all rejected because `reqwest` cannot parse them as absolute URLs.

## `aviso.HttpError: 401`

The configured auth source did not produce credentials the server accepts. Check that:

- `Bearer` was constructed with a token the server's auth backend recognises.
- `Basic` was constructed with the right username and password.
- `Env()` is reading the env vars you think it is (`AVISO_TOKEN`, `AVISO_USERNAME`, `AVISO_PASSWORD`).
- `ConfigFile(...)` points at a file with exactly one of `bearer:` or `basic:` at the top level.

## `aviso.TransportError`

Network failure before the response begins. The error message names the cause (DNS, TCP, TLS). For self-signed certificates in dev, use `aviso.AvisoClient(base_url="...", danger_accept_invalid_certs=True)` and accept that the warning is loud.

## `aviso.HistoryGapError`

A gap was detected in the watch stream. Two reasons:

- `reason == "replay_limit_reached"`: the server's `notification_replay_limit_reached` payload says some of the requested backfill is older than its retention. `.max_allowed` tells you how many notifications can be replayed at most.
- `reason == "sequence_jump"`: the wire delivered a non-consecutive sequence. `.expected` and `.observed` name the boundary.

A gap is terminal: continuing past it would silently violate at-least-once. The client raises and exits the iterator. Decide what the right recovery is (e.g., restart from the live edge with `from_=None`).

## `aviso.TriggerError: command failed`

A required command trigger exited non-zero or timed out. The exception carries:

- `.exit_code`: the child's exit code (`-1` for signal-terminated).
- `.stderr_tail`: the last 4 KiB of the child's stderr.

If the failure is transient, raise the trigger's `retries=` count or set `required=False` to make the watch continue past a failed dispatch.

## `Ctrl+C` does not stop a listening loop

The sync iterator polls the channel every 100 ms and checks for pending signals between polls. If it takes longer than that to respond, you may have a Python operation in the loop body that does not yield. Move heavy work into a background thread or use the async client.

## `aviso.StateStoreError` on first run

`JsonFileStore` does not create parent directories. Create them first:

```python
import pathlib

path = pathlib.Path("~/.config/aviso/state.json").expanduser()
path.parent.mkdir(parents=True, exist_ok=True)

client = aviso.AvisoClient(
    base_url="...",
    state_store=aviso.JsonFileStore(path),
)
```

## Mixing sync and async clients

Do not call sync methods on `AvisoClient` from inside an asyncio event loop. Use `AsyncAvisoClient` instead, or run the sync client in a thread:

```python
import asyncio

result = await asyncio.to_thread(client.notify, event_type="mars", payload={"k": "v"})
```

## Wheels on PyPI

Not available in this release. Install from source with `uv run maturin develop`. When the wheel matrix PR ships, `pip install aviso` becomes the recommended path.
