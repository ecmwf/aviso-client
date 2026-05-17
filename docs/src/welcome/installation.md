# Installation

> Phase 0: not installable yet. This page describes the planned story.

## CLI / Rust library

```bash
cargo install aviso-client-cli  # Phase 4
```

To depend on the library:

```toml
[dependencies]
aviso-client = "0.1"  # Phase 1
```

## Python package

```bash
pip install aviso-client  # Phase 5+
```

## From source

```bash
git clone https://github.com/ecmwf/aviso-client.git
cd aviso-client
cargo build --release            # Rust
uv sync && uv run maturin develop  # Python (Phase 5+)
```
