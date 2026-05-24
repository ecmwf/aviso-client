# Contributing

How to work on this repository. The canonical document is [`CONTRIBUTING.md`](https://github.com/ecmwf/aviso-client/blob/main/CONTRIBUTING.md) at the repository root; this page is a quick orientation for people landing in the book.

## What you need installed

- Rust through [rustup](https://rustup.rs/). The MSRV is in the workspace's root `Cargo.toml`.
- `mdbook` for the documentation: `cargo install mdbook`.
- `mdbook-mermaid` for diagram rendering in the book: `cargo install mdbook-mermaid`. Without it, `mdbook build docs` leaves mermaid blocks as raw code instead of rendered diagrams.
- `cargo-deny` for the license and vulnerability gate: `cargo install cargo-deny`.
- Docker for the end-to-end tests' compose validation. Needed to run the full CI gate set locally; pure cargo workflows do not need it.

The Python toolchain (`uv`, `ruff`, `ty`, `pytest`) is only needed when you touch the future Python extension or the pure-Python helpers; see `CONTRIBUTING.md` for the details.

## The local workflow

```bash
git clone https://github.com/ecmwf/aviso-client.git
cd aviso-client

# Make a change. Run the gates locally before you push:
cargo fmt --all
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo test --locked --workspace --all-targets
cargo test --locked --workspace --doc
mdbook test docs
mdbook build docs
cargo deny check
git diff --exit-code Cargo.lock
docker compose -f tests/e2e/docker-compose.yml config --quiet
```

A pre-commit hook in `.githooks` runs the fast subset (format, clippy, unit tests). Enable it with:

```bash
git config core.hooksPath .githooks
```

A pre-push hook runs the slower targets, including the Docker Compose syntax check.

## What CI runs

The same gates listed above. The full set is in [`.github/workflows/ci.yml`](https://github.com/ecmwf/aviso-client/blob/main/.github/workflows/ci.yml). Every gate is expected to be green on `main`.

## Adding a new trigger

A common contribution. The shape:

1. Add a new payload struct in `crates/aviso/src/watch/trigger/` and wire it into the crate-private `TriggerKind` enum.
2. Add a public constructor on the `Trigger` builder (`Trigger::myname(...)`).
3. Add the dispatcher logic in the trigger module.
4. Add a `MyConfig` payload struct and a `TriggerConfig::MyName(MyConfig)` variant for YAML.
5. Write unit tests for the dispatcher and round-trip serde tests for the YAML.
6. Add a docs page under `docs/src/triggers/` and link it from `docs/src/triggers/overview.md`.

A worked example is in the `webhook` trigger (`crates/aviso/src/watch/trigger/webhook.rs`); it has all the pieces you will need.

## Documentation contributions

Pages live under `docs/src/` in the layout described in [`SUMMARY.md`](https://github.com/ecmwf/aviso-client/blob/main/docs/src/SUMMARY.md). When you add or move a page, keep the SUMMARY in sync.

The voice and the conventions are codified in [Docs style](./docs-style.md). The short version:

- Lead with what the user is trying to do, not with how the code is laid out.
- Examples come early, prose later.
- Avoid being overly technical when there is a plainer way to say the same thing.

## Planning changes

When you propose a structural change, update the relevant planning material instead of burying long rationale in user-facing docs.

The current roadmap is at [`plans/v0.3.md`](https://github.com/ecmwf/aviso-client/blob/main/plans/v0.3.md).

## Filing a bug

Use the [issue tracker](https://github.com/ecmwf/aviso-client/issues). When you can, include:

- The aviso version (`aviso --version`).
- The `X-Request-ID` from the server response (visible in tracing as `request_id`).
- The output of `aviso config dump --redact` if the bug is configuration-shaped.
- Steps to reproduce, against a public server when possible.
