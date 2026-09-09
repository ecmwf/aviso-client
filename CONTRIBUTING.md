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
cargo +1.88.0 check --locked --workspace --all-targets
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

## Continuous integration

CI runs on ECMWF self-hosted Linux runners, inside the `eccr.ecmwf.int/aviso/cli-ci` container image, with Rust compilation cached through sccache. Branch protection requires one status check, `ci-pass`. It passes only when every gated job (Rust, cargo-deny, mdBook, the Python matrix, the e2e compose-config check, the real-stack `e2e` suite, the C++ binding, and the release-tooling lint/tests) succeeds. The `e2e` job runs the Python, Rust, and C++ suites against a real aviso-server + auth-o-tron + NATS stack; because it shares one stack on the self-hosted host, only one `e2e` job runs at a time repo-wide, so it can queue behind another PR's run.

The gated jobs do not run on pull requests from forks. Self-hosted runners must never execute untrusted code, so a fork PR skips them and `ci-pass` stays red. If you open a PR from a fork, expect a maintainer to land your branch inside `ecmwf/aviso-client`, where the full gate runs. The local check set above is the same one CI runs, so a green local run is your best signal that the change will pass.

Documentation publishes on its own. The `Docs Sites Publish` workflow builds the mdBook in the same CI image and pushes it to ECMWF Sites at <https://sites.ecmwf.int/docs/aviso-client>. A push to `main` updates the canonical site that the `latest` link points at; a same-repo pull request gets a preview under `pull-requests/PR-<number>`, with the link posted on the PR and removed when it closes. This flow is not part of `ci-pass`; the `mdBook` gate above is what blocks a merge on a broken docs build.

## Python toolchain

The aviso-py crate builds a `cdylib` extension that the `pyaviso` Python distribution loads as `pyaviso._native`. The `uv` workflow drives every Python-side check:

```bash
uv sync --locked --group dev
uv run maturin develop --locked
uv run ruff check python/ tests/e2e/python/
uv run ruff format --check python/ tests/e2e/python/
uv run ty check python/ tests/e2e/python/
uv run pytest python/tests/
```

`uv sync` materialises a Python virtualenv in `.venv/` from `uv.lock`. `maturin develop --locked` compiles the Rust extension and copies it into the venv's `pyaviso/` package (writes `python/pyaviso/_native*.so` on the system Python and an editable install into `.venv`), and installs the bundled `aviso` console command into `.venv/bin`. The lint, format-check, and type-check commands shown above run against `python/ tests/e2e/python/` as the recommended local superset; the GitHub Actions `python` job currently runs them against `python/` only, so adding `tests/e2e/python/` here protects the e2e suite from drift between local and CI runs.

## End-to-end tests

E2E tests run against a real three-service stack (`aviso-server` + `auth-o-tron` + JetStream-backed NATS) pulled from public registries. Images are pinned by manifest digest in [`tests/e2e/docker-compose.yml`](tests/e2e/docker-compose.yml). The stack mirrors ECMWF's production deployment patterns (auth required, three accounts mapped to three roles, shared JWT secret); see [`tests/e2e/README.md`](tests/e2e/README.md) for the account credentials and the bump procedure.

### Python e2e suite

Assumes the [Python toolchain](#python-toolchain) setup above has already been run (`uv sync` + `uv run maturin develop` builds the `pyaviso._native` extension into the local venv; without it, `uv run pytest tests/e2e/python/` fails on first import).

```bash
bash tests/e2e/shared/stack.sh up
export AVISO_BASE_URL=http://localhost:8000 \
       AVISO_USERNAME=producer-user \
       AVISO_PASSWORD=producer-pass
uv run pytest tests/e2e/python/
```

The stack-up helper polls each service's health endpoint with a 60 s timeout and exits non-zero if any service does not come up. By default the stack is left running after pytest exits so successive runs skip the docker startup cost; set `AVISO_E2E_TEARDOWN=1` to tear down. On any test failure `docker compose logs` are captured to `tests/e2e/last-failure.log` for post-mortem.

The hermetic suite at `python/tests/` stays the default (`uv run pytest`); the e2e suite is opt-in via the explicit path (`tests/e2e/python/`).

### Rust e2e suite

```bash
bash tests/e2e/shared/stack.sh up
cargo build -p aviso-cli
cargo test --locked -p aviso-e2e -- --include-ignored --test-threads=1
bash tests/e2e/shared/stack.sh down  # when done, or `restart` to wipe JetStream between runs
```

Every `#[test]` function in `tests/e2e/rust/` carries `#[ignore = "requires e2e compose stack"]`, so the default `cargo test --workspace` invocation compiles the crate but skips running the tests. The `--include-ignored` flag opts in; `--test-threads=1` keeps publishers and listeners from racing each other in the shared JetStream stream. The CLI tests use `assert_cmd::Command::cargo_bin("aviso")` which expects the `aviso` binary to exist at `target/debug/aviso`, hence the `cargo build -p aviso-cli` step. See `bash tests/e2e/shared/stack.sh` (no args) for the full subcommand list (`up`, `down`, `restart`, `logs`, `status`).

## Releasing

The whole suite releases under one shared version, read from `[workspace.package]` in the root `Cargo.toml`. Every crate inherits it, maturin resolves the Python distribution's version from it, and the bare-semver git tag must equal it. The PyPI distribution `pyaviso` continues the legacy package's version line, which is why the first release is 2.0.0 rather than 0.x.

Local tooling is `just` plus `cargo-release` (`cargo install just cargo-release`). The recipes prepare and tag; CI does every actual publish, so registry credentials never touch a laptop:

```bash
just release-preflight 2.0.3   # local dry-run gate: version consistency, fmt,
                               # clippy, tests, crate packaging, ordered
                               # publish dry-run, wheel + sdist + twine check
just release-version 2.0.3     # bump the workspace and every internal pin
just release-tag 2.0.3         # annotated bare tag + the push command to run
just publish-dry               # launch the CI crates.io publish dry-run via gh
```

The release flow:

1. Run `just release-preflight <version>` locally, then dispatch the
   `Release Preflight` workflow for the same version. Both must be green.
   They publish nothing.
2. Land the version bump from `just release-version` through a normal PR.
3. `just release-tag <version>` on the merged commit, then push the tag with
   `git push origin <version>`.
4. The tag triggers the publishers in parallel: `Publish Crates` (crates.io,
   in dependency order with index polling), `Publish PyPI` (manylinux and
   macOS wheels plus the sdist), `Release Assets` (the prebuilt
   `libaviso_ffi` tarballs attached to a GitHub Release with generated
   notes), and the docs publish (a final release tag moves the `stable`
   docs link; pre-release tags do not).

Every publisher first asserts the release invariant: the tag names the checked-out commit, equals the workspace version, is reachable from `main`, and `ci-pass` was green for that exact commit. A tag pushed at an unreviewed commit publishes nothing.

If a publisher fails partway, never move or re-point the tag: published registry versions are immutable. The crates.io workflow has a resumable retry (dispatch it on the tag with the retry input; already-published crates are skipped only after their checksum matches what the tag's tree packages). A partial PyPI upload means releasing the next patch version. When in doubt, bump the whole workspace and release a fresh tag.

## Code of conduct

Be precise. Be kind. Cite. Push back when something is wrong.
