# Contributing

## Process rules (load-bearing)

1. **Always propose a plan first.** No surprise patches. See [`AGENTS.md`](AGENTS.md) for the full rulebook.
2. **One concern per change.** A PR is a bugfix, a feature, a refactor, or a test pass; not a mix.
3. **Every commit on `main` must pass the full local check set in [Running the checks locally](#running-the-checks-locally) below, which mirrors what CI runs.**
4. **Document features in `docs/` in the same change that introduces them.** Feature without docs is not done.
5. **Never suppress lints/warnings/test failures** unless the suppression itself is the correct semantic choice.

## Planning & tracking

- **GitHub Issues + milestones** are the durable source of truth for in-flight work.
- **`plans/`** holds the project's planning documents (overall plan, roadmap, follow-ups). Phase numbers and roadmap dates live there and nowhere else (see [`AGENTS.md`](AGENTS.md#time-bound-references)).
- **`plans/decisions.md`** is the ADR log. Architectural decisions land there before code that depends on them.

## Branch & commit conventions

- Branches: `<topic>/<slug>`, where `<topic>` is `feat`, `fix`, `refactor`, `docs`, `chore`, or `meta`. Branches do not encode plan ids or dates; plans live under [`plans/`](plans/).
- **Commits follow [Conventional Commits](https://www.conventionalcommits.org/en/v1.0.0/).** The full ruleset lives in [`AGENTS.md`](AGENTS.md#commit-conventions); in short: `<type>(<scope>): <description>`, imperative, ≤72 chars, no period; one concern per commit, sensibly sized, every commit must build and pass the full check set; explain the *why* in the body.

## Running the checks locally

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

This list mirrors the Rust and docs jobs of [`.github/workflows/ci.yml`](.github/workflows/ci.yml). Required tooling: `rustup`, `cargo install mdbook cargo-deny mdbook-mermaid`, and a working Docker (for the compose validation). The mdBook build calls the `mdbook-mermaid` preprocessor for diagram blocks; without it, mermaid diagrams render as raw code blocks. The Python CI job runs the steps in [Python toolchain](#python-toolchain) below.

## Pre-commit / pre-push hooks

The repo ships two git hooks under [`.githooks/`](.githooks/) that automate the gates above:

- **`pre-commit`** runs `cargo fmt --all -- --check` only (sub-second). Refuses to record a commit whose staged Rust files would be rewritten by `cargo fmt --all`. This is the load-bearing gate that prevents the most common CI failure on this repo: editing code, committing, then running `cargo fmt --all` "as a final sanity pass" without re-staging the resulting changes (the working tree ends up formatted but the committed files do not, and the push fails CI minutes later).
- **`pre-push`** runs the full check set above (the same one CI runs). Cold ~30-90s; warm ~5-15s. Optional tooling (`cargo-deny`, `mdbook`, `docker`) is skipped with a notice if absent.

Enable both hooks with a one-time:

```bash
git config core.hooksPath .githooks
```

To skip a single invocation (rarely needed; CI will catch what you skip):

```bash
git commit --no-verify     # skip pre-commit
git push   --no-verify     # skip pre-push
```

`AGENTS.md` requires this gate to be enabled (or the equivalent commands run by hand) before every push. See the [agent rulebook](AGENTS.md#commit-conventions) for the policy text.

## Python toolchain

The aviso-py crate builds a `cdylib` extension that the `aviso` Python distribution loads as `aviso._native`. The `uv` workflow drives every Python-side check:

```bash
uv sync --locked --group dev
uv run maturin develop --locked
uv run ruff check python/
uv run ruff format --check python/
uv run ty check python/
uv run pytest python/tests/
```

`uv sync` materialises a Python virtualenv in `.venv/` from `uv.lock`. `maturin develop --locked` compiles the Rust extension and copies it into the venv's `aviso/` package (writes `python/aviso/_native*.so` on the system Python and an editable install into `.venv`). The remaining four commands are the Python CI gates, run identically by the GitHub Actions `python` job and by the pre-push hook.

## End-to-end tests

E2E tests run against a real `aviso-server` instance pulled from ECMWF's container registry. The pinned version lives in the `image:` tag of [`tests/e2e/docker-compose.yml`](tests/e2e/docker-compose.yml). Bring it up locally with `cd tests/e2e && docker compose up -d`.

## Code of conduct

Be precise. Be kind. Cite. Push back when something is wrong.
