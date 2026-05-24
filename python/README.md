# aviso (Python)

Python client for [`aviso-server`](https://github.com/ecmwf/aviso-server), ECMWF's notification service for data-driven workflows. The repo this package ships from is [`aviso-client`](https://github.com/ecmwf/aviso-client); the installable distribution and the importable module are both named `aviso`.

The package wraps the Rust core via PyO3 bindings. The compiled extension lives at `aviso._native` and a curated Python wrapper at `aviso.__init__` exposes a Pythonic surface: synchronous `AvisoClient` and asynchronous `AsyncAvisoClient`, value types (`Notification`, `NotifyResponse`, `SchemaCatalog`, `SchemaResponse`), the `Trigger` builder, the five auth providers (`Bearer`, `Basic`, `Env`, `ConfigFile`, `Chain`), the two state stores (`MemoryStore`, `JsonFileStore`), and the exception hierarchy rooted at `AvisoError`.

## Install

PyPI wheels land in a follow-up release. Until then, install from a checkout of this repository:

```bash
git clone https://github.com/ecmwf/aviso-client.git
cd aviso-client
uv venv
uv sync --locked --group dev
uv run maturin develop --release --locked
uv run python -c "import aviso; print(aviso.__version__)"
```

## Quickstart

```python
import aviso

client = aviso.AvisoClient(
    base_url="https://aviso.example.org",
    auth=aviso.Bearer("opaque-jwt"),
)

for notification in client.listen("mars", filter={"class": "od"}):
    print(notification.sequence, notification.payload)
```

See the [user-facing documentation](https://github.com/ecmwf/aviso-client/tree/main/docs/src/python) for the full surface, including the async client, triggers, auth providers, state stores, and the exception hierarchy.

## License

Apache-2.0. See [`LICENSE.txt`](https://github.com/ecmwf/aviso-client/blob/main/LICENSE.txt). Copyright 2026 ECMWF and individual contributors.
