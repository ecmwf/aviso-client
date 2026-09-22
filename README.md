<!--
SPDX-FileCopyrightText: 2026 European Centre for Medium-Range Weather Forecasts (ECMWF)
SPDX-License-Identifier: Apache-2.0
-->

<div align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="https://raw.githubusercontent.com/ecmwf/logos/cde127b2c872e88474570a681e56b14cdecf4f03/logos/aviso/aviso_text_dark.svg">
    <source media="(prefers-color-scheme: light)" srcset="https://raw.githubusercontent.com/ecmwf/logos/cde127b2c872e88474570a681e56b14cdecf4f03/logos/aviso/aviso_text_light.svg">
    <img alt="Aviso Logo" src="https://raw.githubusercontent.com/ecmwf/logos/cde127b2c872e88474570a681e56b14cdecf4f03/logos/aviso/aviso_text_light.svg">
  </picture>
</div>

<p align="center">
  <a href="https://crates.io/crates/aviso">
    <img src="https://img.shields.io/crates/v/aviso.svg" alt="crates.io">
  </a>
  <a href="https://pypi.org/project/pyaviso/">
    <img src="https://img.shields.io/pypi/v/pyaviso.svg" alt="PyPI">
  </a>
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

## Installation

Python package and the `aviso` CLI (wheels for Linux x86_64/aarch64 and macOS, Python 3.10+):

```bash
pip install pyaviso
aviso --version
```

Rust:

```bash
cargo add aviso               # the client library
cargo install aviso-cli       # the aviso command-line tool
```

C and C++: download the prebuilt `libaviso_ffi` tarball for your platform (headers, static and shared library, pkg-config file) from the [GitHub Releases](https://github.com/ecmwf/aviso-client/releases), or build the `aviso-ffi` crate from source with [cargo-c](https://github.com/lu-zero/cargo-c). See the [C++ binding docs](https://sites.ecmwf.int/docs/aviso-client/main/cpp/overview.html).

## Repository layout

- **`crates/aviso`**: Rust library crate (the core implementation; published as `aviso` on crates.io).
- **`crates/aviso-cli`**: Rust binary crate producing the `aviso` command-line tool.
- **`crates/aviso-py`**: PyO3 binding crate. Builds as a `cdylib` extension named `pyaviso._native` plus an `rlib` so the workspace's `cargo test` sees its types.
- **`python/pyaviso/`**: pure-Python wrapper around `pyaviso._native`. The installable distribution and the importable module are both named `pyaviso`. The wheel also bundles the `aviso` CLI as a console command (a script that runs the Rust CLI through the extension), so `pip install pyaviso` provides both `import pyaviso` and the `aviso` command. Published to PyPI as [`pyaviso`](https://pypi.org/project/pyaviso/); built locally with `uv run maturin develop`.
- **`crates/aviso-ffi`**: C/C++ binding crate. Builds `libaviso_ffi` (a `staticlib` and a `cdylib`) exposing a stable C ABI through a `cbindgen`-generated header (`include/aviso.h`), plus a hand-written header-only C++ facade (`include/aviso.hpp`) with RAII handles and a throwing `aviso::Error`. A C or C++ application links the prebuilt library with its own toolchain and needs no Rust toolchain in its build.
- **`examples/cpp/`**: worked C++ consumers of the binding, built with CMake against the library and facade. They double as the binding's tested reference.
- **`docs/`**: mdBook user-facing documentation (CLI, Rust library, Python package, C++ binding).

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
shellcheck tests/e2e/shared/stack.sh tests/e2e/shared/stack.test.sh
bash tests/e2e/shared/stack.test.sh
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

C++ binding (needs CMake and a C++17 compiler; the generated header is checked in and guarded against drift):

```bash
cargo build --locked -p aviso-ffi
cargo run --locked -p aviso-ffi --features gen-header --bin gen-header
git diff --exit-code crates/aviso-ffi/include/aviso.h   # header is up to date
cmake -S examples/cpp -B build/cpp -DAVISO_FFI_LIB_DIR="$PWD/target/debug"
cmake --build build/cpp
./build/cpp/01_schema                                   # runs without a server
```

Release tooling (the release-invariant check that gates every publisher, plus the preflight's version-consistency, ordered dry-run, and artifact-packaging checks; shell plus a C/C++ compiler and pkg-config for the packaging suite, no Rust toolchain):

```bash
shellcheck scripts/*.sh scripts/tests/*.test.sh
bash scripts/tests/release-invariant.test.sh
bash scripts/tests/version-consistency.test.sh
bash scripts/tests/publish-dry-run.test.sh
bash scripts/tests/publish-crates.test.sh
bash scripts/tests/package-ffi.test.sh
```

## Documentation

The full documentation is hosted at <https://sites.ecmwf.int/docs/aviso-client/main/>. Good starting points:

- [Getting started](docs/src/getting-started/overview.md) for install and first steps.
- [Command-line interface](docs/src/cli/quickstart.md) for the `aviso` CLI.
- [Python package](docs/src/python/overview.md) for the `pyaviso` library.
- [C++ binding](docs/src/cpp/overview.md) for the `aviso-ffi` C ABI and C++ facade.

To build and preview the book locally:

```bash
cargo install mdbook mdbook-mermaid
mdbook serve docs --open
```

## Working in this repo

See [`CONTRIBUTING.md`](CONTRIBUTING.md). The agent rulebook is [`AGENTS.md`](AGENTS.md). Durable planning lives in GitHub Issues + milestones.

## License

[Apache License 2.0](LICENSE)

In applying this licence, ECMWF does not waive the privileges and immunities
granted to it by virtue of its status as an intergovernmental organisation
nor does it submit to any jurisdiction.

Copyright 2026 ECMWF and individual contributors.
