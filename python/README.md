# pyaviso

Python client for [`aviso-server`](https://github.com/ecmwf/aviso-server), ECMWF's notification service for data-driven workflows. The repo this package ships from is [`aviso-client`](https://github.com/ecmwf/aviso-client); the installable distribution and the importable module are both named `pyaviso`.

The package wraps the Rust core via PyO3 bindings. The compiled extension lives at `pyaviso._native` and a curated Python wrapper at `pyaviso.__init__` exposes a Pythonic surface: synchronous `AvisoClient` and asynchronous `AsyncAvisoClient`, value types (`Notification`, `NotifyResponse`, `SchemaCatalog`, `SchemaResponse`), the `Trigger` builder, the five auth providers (`Bearer`, `Basic`, `Env`, `ConfigFile`, `Chain`), the two state stores (`MemoryStore`, `JsonFileStore`), and the exception hierarchy rooted at `AvisoError`.

Installing `pyaviso` also puts the `aviso` command-line tool on your PATH, so one install gives you both the importable library and the CLI.

## Install

PyPI wheels are not published yet. Install from a checkout of this repository:

```bash
git clone https://github.com/ecmwf/aviso-client.git
cd aviso-client
uv venv
uv sync --locked --group dev
uv run maturin develop --release --locked
uv run python -c "import pyaviso; print(pyaviso.__version__)"
uv run aviso --version
```

## Quickstart

Set two environment variables for the server URL and credentials, then run a listener. `pyaviso.Env()` reads `AVISO_TOKEN` (preferred) or the `AVISO_USERNAME`/`AVISO_PASSWORD` pair.

```bash
export AVISO_BASE_URL=https://aviso.example.org
export AVISO_USERNAME=alice
export AVISO_PASSWORD=wonderland
```

```python
import os
import pyaviso

client = pyaviso.AvisoClient(base_url=os.environ["AVISO_BASE_URL"], auth=pyaviso.Env())

for notification in client.listen("test_polygon", filter={"polygon": "0,0,1,0,1,1,0,0"}):
    print(notification.sequence, notification.payload)
```

The example uses an event type called `test_polygon`. Substitute your own if your server has different schemas configured; discover what is there with `client.schema().event_types` and `client.schema_for("<event_type>")`. The [user-facing Python documentation](https://github.com/ecmwf/aviso-client/tree/main/docs/src/python) covers the full surface, including the async client, triggers, state stores, the exception hierarchy, and how schemas work.

## License

Apache-2.0. See [`LICENSE.txt`](https://github.com/ecmwf/aviso-client/blob/main/LICENSE.txt). Copyright 2026 ECMWF and individual contributors.
