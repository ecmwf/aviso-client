# Install

The Python package ships source-only in this release. PyPI wheels land in a follow-up; until then, the install path is a checkout of the repository plus `maturin develop`.

## From source

```bash
git clone https://github.com/ecmwf/aviso-client.git
cd aviso-client
uv venv
uv sync --locked --group dev
uv run maturin develop --release --locked
```

The `--group dev` flag pulls in `maturin`, `ruff`, `ty`, `pytest`, and the other dev tools alongside the runtime dependencies. `--locked` on both commands keeps the install reproducible from the committed `uv.lock` and `Cargo.lock`. The second command builds the Rust extension into the local virtualenv so `import aviso` works. The first build pulls in the workspace's Rust dependencies and compiles them; subsequent builds are incremental.

You need:

- Rust toolchain (`rustup`).
- `uv` ([install instructions](https://docs.astral.sh/uv/)).
- A C compiler and linker on the path. macOS uses the Apple toolchain; on Linux the system `gcc` or `clang` is fine.

## Verifying the install

```bash
uv run python -c "import aviso; print(aviso.__version__)"
```

Expected: a version string like `0.1.0` (matching the Rust workspace version).

## From PyPI (coming soon)

<!-- not-runnable -->
```bash
pip install aviso
```

The wheel matrix (manylinux, macOS universal2, Windows) lands in a separate PR. Once published, `pip install aviso` will be the recommended path.

## Python version

`aviso` targets Python 3.10 and newer. The extension is built with `abi3-py310`, so a single wheel works across every supported Python version on the same platform.
