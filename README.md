# aviso-client

Client suite for [`aviso-server`](https://github.com/ecmwf/aviso-server), ECMWF's notification service for data-driven workflows.

## Repository layout

- **`crates/aviso`**: Rust library crate (the core implementation; published as `aviso` on crates.io).
- **`crates/aviso-cli`**: Rust binary crate producing the `aviso` command-line tool.
- **`crates/aviso-py`**: PyO3 binding crate. Builds as a `cdylib` extension named `aviso._native` plus an `rlib` so the workspace's `cargo test` sees its types.
- **`python/aviso/`**: pure-Python wrapper around `aviso._native`. The installable distribution and the importable module are both named `aviso`. Built locally with `uv run maturin develop`; PyPI wheels are not published yet, so install from a checkout.
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
uv run ruff check python/
uv run ruff format --check python/
uv run ty check python/
uv run pytest python/tests/
```

The Python toolchain (`uv`, `ruff`, `ty`, `pytest`, `maturin`) is documented in [`CONTRIBUTING.md`](CONTRIBUTING.md).

## End-to-end tests

A real three-service stack (`aviso-server` + `auth-o-tron` + JetStream-backed NATS) lives at [`tests/e2e/`](tests/e2e/) for behaviour the hermetic suites cannot reach (real-wire reconnect, auth + role enforcement, CLI subcommands). `bash tests/e2e/shared/stack.sh up` brings the stack up (then `down`, `restart`, `logs`, or `status` for the rest of the lifecycle). Then run the Python suite via `uv run pytest tests/e2e/python/` and the Rust suite via `cargo build -p aviso-cli && cargo test -p aviso-e2e -- --include-ignored --test-threads=1` (the build step is required because the CLI tests expect `target/debug/aviso`). See [`tests/e2e/README.md`](tests/e2e/README.md) for the test accounts and the bump procedure for the pinned container images; see [`CONTRIBUTING.md`](CONTRIBUTING.md#end-to-end-tests) for the full local workflow.

## Documentation

```bash
cargo install mdbook mdbook-mermaid
mdbook serve docs --open
```

The Python user-facing pages live under [`docs/src/python/`](docs/src/python/) and start at [`overview.md`](docs/src/python/overview.md).

## Project plan

The current plan and roadmap live under [`plans/`](plans/). Architectural decisions (the *why* of each design choice) live in [`plans/decisions.md`](plans/decisions.md) and are referenced from plans by their stable ADR id.

## Working in this repo

See [`CONTRIBUTING.md`](CONTRIBUTING.md). The agent rulebook is [`AGENTS.md`](AGENTS.md). Durable planning lives in GitHub Issues + milestones; planning documents live in [`plans/`](plans/).

## License

Apache-2.0. See [`LICENSE.txt`](LICENSE.txt). Copyright 2026 ECMWF and individual contributors.
