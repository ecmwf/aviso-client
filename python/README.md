# aviso (Python)

Python client for [`aviso-server`](https://github.com/ecmwf/aviso-server). The repo this package ships from is [`aviso-client`](https://github.com/ecmwf/aviso-client); the installable distribution and the importable module are both named `aviso`.

> Status: **Phase 0 — package skeleton only.** No public functionality yet. `maturin develop` is **not** usable until Phase 5 adds the PyO3 binding code and switches the `aviso-py` crate to `cdylib`.

## Install (Phase 5+)

```bash
pip install aviso
```

## Develop locally (Phase 5+)

```bash
uv sync
uv run maturin develop --release
uv run python -c "import aviso; print(aviso.__version__)"
```

## Roadmap

See [`docs/src/internals/decisions.md`](../docs/src/internals/decisions.md) for the phased roadmap.
