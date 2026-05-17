# Contributing

## Process rules (load-bearing)

1. **Always propose a plan first.** No surprise patches. See [`AGENTS.md`](AGENTS.md) for the full rulebook.
2. **One concern per change.** A PR is a bugfix, a feature, a refactor, or a test pass; not a mix.
3. **Every commit on `main` must build and pass `cargo fmt`, `cargo clippy -- -D warnings`, `cargo test --workspace`, `mdbook build docs`, and `cargo deny check`.**
4. **Document features in `docs/` in the same change that introduces them.** Feature without docs is not done.
5. **Never suppress lints/warnings/test failures** unless the suppression itself is the correct semantic choice.

## Planning & tracking

- **GitHub Issues + milestones** are the durable source of truth for in-flight work.
- **`plans/`** holds the project's planning documents (overall plan, roadmap, follow-ups). Phase numbers and roadmap dates live there and nowhere else (see [`AGENTS.md`](AGENTS.md#time-bound-references)).
- **`docs/src/internals/decisions.md`** is the ADR log. Architectural decisions land there before code that depends on them.

## Branch & commit conventions

- Branches: `<topic>/<slug>`, where `<topic>` is `feat`, `fix`, `refactor`, `docs`, `chore`, or `meta`. Branches do not encode plan ids or dates; plans live under [`plans/`](plans/).
- **Commits follow [Conventional Commits](https://www.conventionalcommits.org/en/v1.0.0/).** The full ruleset lives in [`AGENTS.md`](AGENTS.md#commit-conventions); in short: `<type>(<scope>): <description>`, imperative, ≤72 chars, no period; one concern per commit, sensibly sized, every commit must build and pass the full check set; explain the *why* in the body.

## Running the checks locally

```bash
cargo fmt --all -- --check
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo test --locked --workspace --all-targets
mdbook build docs
cargo deny check
```

Python toolchain (once `python/aviso/` carries code):

```bash
uv sync
uv run ruff check python/
uv run ruff format --check python/
uv run ty check python/
uv run pytest python/tests/
```

## End-to-end tests

E2E tests run against a real `aviso-server` instance pulled from ECMWF's container registry. The pinned version lives in the `image:` tag of [`tests/e2e/docker-compose.yml`](tests/e2e/docker-compose.yml). Bring it up locally with `cd tests/e2e && docker compose up -d`.

## Code of conduct

Be precise. Be kind. Cite. Push back when something is wrong.
