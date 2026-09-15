# Install

Use Python 3.10 or newer. Install into the Python environment where you will run
your scripts or notebook kernel.

## From PyPI

```bash
pip install pyaviso
```

Wheels ship for Linux (manylinux, x86_64 and aarch64) and macOS (universal2).
No Rust toolchain is needed; the compiled extension comes inside the wheel.

## Verifying the install

```bash
python -c "import pyaviso; print(pyaviso.__version__)"
```

It prints the installed package version. This checks the import, not a
connection to an Aviso server.

## The bundled `aviso` command

Installing `pyaviso` also puts the `aviso` command-line tool on your PATH, so a
single `pip install` gives you both the importable library and the CLI:

```bash
aviso --version
```

It is the same `aviso` command-line tool documented in the
[CLI section](../cli/overview.md). The wheel installs it as a console script
that runs the Rust CLI in-process through the extension (rather than shipping a
separate compiled binary), so you do not need a `cargo install`. Pick whichever
install path you prefer: `pip install pyaviso` and `cargo install aviso-cli`
give you the same `aviso` command and behaviour.

## From source

To build a checkout locally, including when contributing:

```bash
git clone https://github.com/ecmwf/aviso-client.git
cd aviso-client
uv venv
uv sync --locked --group dev
uv run maturin develop --release --locked
```

The `--group dev` flag pulls in `maturin`, `ruff`, `ty`, `pytest`, and the other
dev tools alongside the runtime dependencies. `--locked` on both commands keeps
the install reproducible from the committed `uv.lock` and `Cargo.lock`. The
second command builds the Rust extension into the local virtualenv so
`import pyaviso` works. The first build pulls the workspace's Rust dependencies
and compiles them; subsequent builds are incremental.

For a source build you need:

- Rust toolchain (`rustup`).
- `uv` ([install instructions](https://docs.astral.sh/uv/)).
- A C compiler and linker on the path. macOS uses the Apple toolchain; on Linux
  the system `gcc` or `clang` is fine.

## Python version

`pyaviso` targets Python 3.10 and newer. The extension is built with
`abi3-py310`, so a single wheel works across every supported Python version on
the same platform.

## What next

- [Quickstart](./quickstart.md) shows how to discover schemas, listen for
  notifications, and replay history.
- [Overview](./overview.md) is the page-by-page guide to what the package gives
  you.
