# Installation

This page describes how each surface of the suite is installed. Surfaces that have not yet shipped functionality are still installable as scaffold; consult [`plans/`](https://github.com/ecmwf/aviso-client/tree/main/plans) for what each surface currently implements.

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

```bash
pip install aviso
```

## From source

```bash
git clone https://github.com/ecmwf/aviso-client.git
cd aviso-client
cargo build --release                # Rust
uv sync && uv run maturin develop    # Python
```
