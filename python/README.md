# aviso (Python)

Python client for [`aviso-server`](https://github.com/ecmwf/aviso-server). The repo this package ships from is [`aviso-client`](https://github.com/ecmwf/aviso-client); the installable distribution and the importable module are both named `aviso`.

> Status: package skeleton only. `maturin develop` is not yet usable because the `aviso-py` crate is currently an `rlib` rather than a `cdylib`.

## Install

```bash
pip install aviso
```

## Develop locally

```bash
uv sync
uv run maturin develop --release
uv run python -c "import aviso; print(aviso.__version__)"
```

## Roadmap

See [`plans/`](../plans/) for the current plan and roadmap, and [`docs/src/internals/decisions.md`](../docs/src/internals/decisions.md) for the architectural decisions.
