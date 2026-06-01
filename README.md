<div align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="https://raw.githubusercontent.com/ecmwf/logos/cde127b2c872e88474570a681e56b14cdecf4f03/logos/aviso/aviso_text_dark.svg">
    <source media="(prefers-color-scheme: light)" srcset="https://raw.githubusercontent.com/ecmwf/logos/cde127b2c872e88474570a681e56b14cdecf4f03/logos/aviso/aviso_text_light.svg">
    <img alt="Aviso Logo" src="https://raw.githubusercontent.com/ecmwf/logos/cde127b2c872e88474570a681e56b14cdecf4f03/logos/aviso/aviso_text_light.svg">
  </picture>
</div>

<p align="center">
  <a href="https://sites.ecmwf.int/docs/aviso-client/main/">
    <img src="https://img.shields.io/badge/docs-online-blue" alt="Docs Badge">
  </a>
  <a href="https://github.com/ecmwf/codex/raw/refs/heads/main/ESEE">
    <img src="https://github.com/ecmwf/codex/raw/refs/heads/main/ESEE/foundation_badge.svg" alt="Foundation Badge">
  </a>
  <a href="https://github.com/ecmwf/codex/raw/refs/heads/main/Project%20Maturity">
    <img src="https://github.com/ecmwf/codex/raw/refs/heads/main/Project%20Maturity/emerging_badge.svg" alt="Maturity Badge">
  </a>
</p>

> [!IMPORTANT]
> This software is **Emerging** and subject to ECMWF's guidelines on [Software Maturity](https://github.com/ecmwf/codex/raw/refs/heads/main/Project%20Maturity).

# aviso-client

Client suite for [`aviso-server`](https://github.com/ecmwf/aviso-server), ECMWF's notification service for data-driven workflows. The full documentation is hosted at <https://sites.ecmwf.int/docs/aviso-client/main/>.

## Repository layout

- **`crates/aviso`**: Rust library crate (the core implementation; published as `aviso` on crates.io).
- **`crates/aviso-cli`**: Rust binary crate producing the `aviso` command-line tool.
- **`crates/aviso-py`**: PyO3 binding crate. Builds as a `cdylib` extension named `pyaviso._native` plus an `rlib` so the workspace's `cargo test` sees its types.
- **`python/pyaviso/`**: pure-Python wrapper around `pyaviso._native`. The installable distribution and the importable module are both named `pyaviso`. The wheel also bundles the `aviso` CLI as a console command (a script that runs the Rust CLI through the extension), so `pip install pyaviso` provides both `import pyaviso` and the `aviso` command. Built locally with `uv run maturin develop`; PyPI wheels are not published yet, so install from a checkout.
- **`docs/`**: mdBook user-facing documentation (CLI, Rust library, Python package).

## Running the full check set

These are the same commands CI runs (see [`.github/workflows/ci.yml`](.github/workflows/ci.yml)). A fresh clone passes them after installing `rustup`, `cargo install mdbook cargo-deny mdbook-mermaid`, [`uv`](https://docs.astral.sh/uv/), and a working Docker (for the compose validation).

Rust and docs:

```bash
cargo fmt --all -- --check
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo build --locked --workspace --all-targets
cargo test --locked --workspace --all-targets
cargo test --locked --workspace --doc
git diff --exit-code Cargo.lock
cargo deny check
mdbook build docs
mdbook test docs
docker compose -f tests/e2e/docker-compose.yml config --quiet
```

Python:

```bash
uv sync --locked --group dev
uv run maturin develop --locked
uv run aviso --version            # the wheel bundles the aviso CLI as a console command
uv run ruff check python/
uv run ruff format --check python/
uv run ty check python/
uv run pytest python/tests/
```

The Python toolchain (`uv`, `ruff`, `ty`, `pytest`, `maturin`) is documented in [`CONTRIBUTING.md`](CONTRIBUTING.md).

## Documentation

The full documentation is hosted at <https://sites.ecmwf.int/docs/aviso-client/main/>. Good starting points:

- [Getting started](docs/src/getting-started/overview.md) for install and first steps.
- [Command-line interface](docs/src/cli/quickstart.md) for the `aviso` CLI.
- [Python package](docs/src/python/overview.md) for the `pyaviso` library.

To build and preview the book locally:

```bash
cargo install mdbook mdbook-mermaid
mdbook serve docs --open
```

## Working in this repo

See [`CONTRIBUTING.md`](CONTRIBUTING.md). The agent rulebook is [`AGENTS.md`](AGENTS.md). Durable planning lives in GitHub Issues + milestones; planning documents live in [`plans/`](plans/).

## License

Apache-2.0. See [`LICENSE.txt`](LICENSE.txt). Copyright 2026 ECMWF and individual contributors.
