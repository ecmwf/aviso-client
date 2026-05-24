# aviso-client

Client suite for [`aviso-server`](https://github.com/ecmwf/aviso-server), ECMWF's notification service for data-driven workflows.

## What's in the box

- **`crates/aviso`**: Rust library crate (the core implementation; published as `aviso` on crates.io).
- **`crates/aviso-cli`**: Rust binary crate producing the `aviso` command-line tool.
- **`crates/aviso-py`**: PyO3 binding crate. Builds as a `cdylib` extension named `aviso._native` plus an `rlib` so the workspace's `cargo test` sees its types.
- **`python/aviso/`**: pure-Python wrapper around `aviso._native`. The installable distribution and the importable module are both named `aviso`. Built locally with `uv run maturin develop` (PyPI wheels land in a follow-up).
- **`docs/`**: mdBook user-facing documentation.

## Build

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

This is the full set CI runs (see [`.github/workflows/ci.yml`](.github/workflows/ci.yml)) and every command is expected to be green on `main`. A fresh clone passes it after installing `rustup`, `cargo install mdbook cargo-deny`, and a working Docker (for the compose validation). The Python toolchain (`uv`, `ruff`, `ty`, `pytest`) is described in [`CONTRIBUTING.md`](CONTRIBUTING.md).

## Documentation

```bash
cargo install mdbook
mdbook serve docs --open
```

## Project plan

The current plan and roadmap live under [`plans/`](plans/). Architectural decisions (the *why* of each design choice) live in [`plans/decisions.md`](plans/decisions.md) and are referenced from plans by their stable ADR id.

## Working in this repo

See [`CONTRIBUTING.md`](CONTRIBUTING.md). The agent rulebook is [`AGENTS.md`](AGENTS.md). Durable planning lives in GitHub Issues + milestones; planning documents live in [`plans/`](plans/).

## License

Apache-2.0. See [`LICENSE.txt`](LICENSE.txt). Copyright 2026 ECMWF and individual contributors.
