# aviso (Python)

Python client for [`aviso-server`](https://github.com/ecmwf/aviso-server). The repo this package ships from is [`aviso-client`](https://github.com/ecmwf/aviso-client); the installable distribution and the importable module are both named `aviso`.

> Status: package skeleton only. The package is not yet installable. The `aviso-py` crate is currently a Rust `rlib` rather than a Python `cdylib`, so `maturin develop` and `pip install` do not produce a working module; both will start working once the PyO3 bindings land.

## Install (once bindings land)

```bash
pip install aviso
```

## Develop locally (once bindings land)

```bash
uv sync
uv run maturin develop --release
uv run python -c "import aviso; print(aviso.__version__)"
```

## Roadmap

See [`plans/`](../plans/) for the current plan and roadmap, and [`docs/src/internals/decisions.md`](../docs/src/internals/decisions.md) for the architectural decisions.
