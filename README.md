# aviso-client

Client suite for [`aviso-server`](https://github.com/ecmwf/aviso-server), ECMWF's notification service for data-driven workflows.

> Status: **Phase 0 — repository bootstrap.** No public functionality yet.

## What's in the box

- **`crates/aviso`** — Rust library crate (the core implementation; published as `aviso` on crates.io).
- **`crates/aviso-cli`** — Rust binary crate producing the `aviso` command-line tool.
- **`crates/aviso-py`** — PyO3 extension crate exposing the library to Python.
- **`python/aviso/`** — Pure-Python helpers shipping alongside the extension; the installable distribution and the importable module are both named `aviso`.
- **`docs/`** — mdBook user-facing documentation.

## Build

```bash
cargo fmt --all -- --check
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo test --locked --workspace --all-targets
mdbook build docs
cargo deny check
```

All five commands are expected to be green on `main`. Phase 0's only acceptance criterion is that a fresh clone passes them without manual setup beyond `rustup` and `cargo install mdbook cargo-deny`.

## Documentation

```bash
cargo install mdbook
mdbook serve docs --open
```

## Project plan

The phased roadmap lives in [`docs/src/internals/decisions.md`](docs/src/internals/decisions.md). Each phase has explicit scope, out-of-scope, "done" criteria, and documentation expectations. Phase 0 (this PR) only delivers structure; Phase 1 onwards delivers behaviour.

## Working in this repo

See [`CONTRIBUTING.md`](CONTRIBUTING.md). The agent rulebook is [`AGENTS.md`](AGENTS.md). Active local notes live in [`TODO.md`](TODO.md); durable planning lives in GitHub Issues + milestones.

## License

Apache-2.0. See [`LICENSE.txt`](LICENSE.txt). Copyright 2026 ECMWF and individual contributors.
