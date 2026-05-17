# Installation

This page describes how each surface of the suite is installed once it ships. Surfaces that have not yet shipped functionality are listed below; consult [`plans/`](https://github.com/ecmwf/aviso-client/tree/main/plans) for what each surface currently implements.

## CLI / Rust library

```bash
cargo install aviso-cli              # installs the `aviso` binary
```

To depend on the library:

```toml
[dependencies]
aviso = "0.1"
```

## Python package

Not yet installable. The `aviso-py` crate is currently a Rust `rlib`, not a Python `cdylib`, so both `pip install aviso` and `uv run maturin develop` fail. Both start working once the PyO3 bindings land.

## From source

```bash
git clone https://github.com/ecmwf/aviso-client.git
cd aviso-client
cargo build --release                # Rust workspace
```

A Python from-source workflow (`uv sync && uv run maturin develop`) lands together with the PyO3 bindings; until then the Rust crates are the only buildable surface.
