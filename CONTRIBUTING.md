# Contributing

## Process rules (load-bearing)

1. **Always propose a plan first.** No surprise patches. See [`AGENTS.md`](AGENTS.md) for the full rulebook.
2. **One concern per change.** A PR is a bugfix, a feature, a refactor, or a test pass — not a mix.
3. **Every commit on `main` must build and pass `cargo fmt`, `cargo clippy -- -D warnings`, `cargo test --workspace`, `mdbook build docs`, and `cargo deny check`.**
4. **Document features in `docs/` in the same change that introduces them.** Feature without docs is not done.
5. **Never suppress lints/warnings/test failures** unless the suppression itself is the correct semantic choice.

## Planning & tracking

- **GitHub Issues + milestones** are the durable source of truth for in-flight work.
- **`TODO.md`** at the root is *optional and short*; use it for active local notes during a working session. It is not a substitute for an issue.
- **`docs/src/internals/decisions.md`** is the ADR log. Architectural decisions land there before code that depends on them.

## Branch & commit conventions

- Branches: `phase-N-<slug>` for plan-aligned work; `fix/<slug>`, `refactor/<slug>`, `docs/<slug>` for everything else.
- Commit subject: imperative, ≤72 chars, no period.
- Commit body: the *why*. If the change is non-obvious, explain in prose. Cite issue numbers where applicable.

## Running the checks locally

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
mdbook build docs
cargo deny check
```

Python toolchain (once Phase 5 lands and `python/aviso_client/` carries code):

```bash
uv sync
uv run ruff check python/
uv run ruff format --check python/
uv run ty check python/
uv run pytest python/tests/
```

## End-to-end tests

E2E tests run against a real `aviso-server` instance pinned to a specific commit. The pinned SHA lives in [`tests/e2e/.env`](tests/e2e/.env). Bring it up locally with `cd tests/e2e && docker compose up -d`.

## Code of conduct

Be precise. Be kind. Cite. Push back when something is wrong.
