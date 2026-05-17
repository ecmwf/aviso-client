# aviso-client (Python)

Python client for [`aviso-server`](https://github.com/ecmwf/aviso-server).

> Status: **Phase 0 — package skeleton only.** No public functionality yet.

## Install (Phase 5+)

```bash
pip install aviso-client
```

## Develop locally

```bash
uv sync
uv run maturin develop --release
uv run python -c "import aviso_client; print(aviso_client.__version__)"
```

## Roadmap

See [`docs/src/internals/decisions.md`](../docs/src/internals/decisions.md) for the phased roadmap.
