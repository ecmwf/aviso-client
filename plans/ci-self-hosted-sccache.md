# CI on self-hosted runners with sccache

Moves the Linux CI load off GitHub-hosted runners onto ECMWF self-hosted runners, inside a project-owned container image, and adds an S3-backed `sccache` so cold Rust builds become incremental across runs and runners. Modelled on the `ecmwf/tensogram` CI (same infra, same secrets).

No release or publish work is in scope here. This plan supersedes the sketch in [`e2e-ci-self-hosted.md`](./e2e-ci-self-hosted.md): besides the cheap `docker compose config` pre-check, it also adds the real-stack `e2e` job (see "e2e suite" below), which runs the full suites against live services but stays informational (not in `ci-pass`) until a flake-free soak.

## Decisions taken

- **Linux only.** No macOS, no Windows in everyday CI. macOS regressions will surface at PyPI-release time, not before; the stack (rustls, tokio, reqwest, no platform syscalls) makes that risk low and the trade deliberate.
- **Same runners as tensogram:** `[self-hosted, Linux, platform-builder-docker-xl, platform-builder-Ubuntu-22.04]`.
- **No CI on external fork PRs.** Self-hosted runners never execute untrusted fork code. The contributor flow is: a maintainer lands external work on a branch in `ecmwf/aviso-client`, where it runs as a same-repo push and gets the full gate. Fork PRs therefore cannot merge directly (the required `ci-pass` check never goes green for them).
- **Doc/lint tools are baked into the image** (`mdbook`, `mdbook-mermaid`, `cargo-deny`) at latest-as-of-image-build. Refreshing them = bump `.github/ci/VERSION` and rebuild. This is an owner-approved exception to the AGENTS.md "no version pin for build tools" rule: the image VERSION is the update lever.

## Release note (out of scope, recorded so it is not re-derived)

- **crates.io** needs no macOS, ever: `cargo publish` ships source and builds from Linux.
- **PyPI** binary wheels need a macOS runner at release time (Apple Silicon `arm64` via `macos-14` and/or Intel `x86_64` via `macos-13`, or a `universal2` build). That belongs in a future release workflow triggered on tags, where GitHub-hosted macOS minutes are negligible. No Windows wheels wanted.

## The CI image

New files: `.github/ci/Dockerfile`, `.github/ci/VERSION`, `.github/workflows/ci-image.yml`. Image name `eccr.ecmwf.int/aviso/cli-ci`.

Ubuntu 24.04 base, single stage (plus a stage to copy the `uv` binary). ~1.1 GB uncompressed locally, **~369 MiB compressed in eccr** (smaller than tensogram's 500-600 MiB, which also carries node/wasm/cmake). Bakes:

- Rust toolchain **1.95.0** (matches `rust-toolchain.toml`) + clippy + rustfmt, with `rustup default 1.95.0` set so directly-wrapped `rustc` invocations (maturin) resolve the right toolchain without a `rust-toolchain.toml` lookup from the wrong directory.
- `sccache`, `cargo-deny`, `mdbook`, `mdbook-mermaid` as **prebuilt static (musl) binaries**, version-pinned via `ARG` (smaller than compiling from source and the image build stays in seconds). Refresh = bump the `ARG` + `.github/ci/VERSION`. This is the owner-approved "bake + update from time to time" mechanism.
- `uv` (version-specific interpreters for the binding matrix installed at CI runtime by uv).
- `gcc`, `libc6-dev`, `git`, `curl`, `ca-certificates`, `python3-dev`. **A C compiler is required:** `ring` 0.17 and `cc` are in `Cargo.lock` (ring drives gcc directly — no perl/make/cmake). **A system Python is required:** `aviso-py` uses pyo3 `abi3` + `auto-initialize` with `extension-module` off for plain cargo, so workspace `cargo build/clippy/test` need an interpreter and libpython (this bit the first `0.1.0` run, whose Rust job had no Python). No g++, no pkg-config, no OpenSSL (rustls), no node, no wasm, no cmake. Stripping the toolchain `.so`s was tried and rejected: it saved 15 MB and segfaulted cargo.

`ci-image.yml`: builds and pushes on changes to `.github/ci/**`, **plus `rust-toolchain.toml` and the workflow file itself** (a toolchain bump must not silently desync the image). Tags full/minor/major from `VERSION`, `cache-from`/`cache-to: type=gha`, ECCR login via `ECMWF_DOCKER_REGISTRY_USERNAME` / `ECMWF_DOCKER_REGISTRY_ACCESS_TOKEN`. Push only on non-PR events.

## The sccache bucket

Dedicated bucket `aviso-client-ci-cache` on the ECMWF object store (endpoint `object-store.os-api.cci1.ecmwf.int`, region `us-east-1`), under key prefix `aviso-client/`. **Already provisioned** with the existing `S3_ACCESS_KEY_ID` / `S3_SECRET_ACCESS_KEY` creds (the one-off creation script has been removed). The object store rejected a server-side expiry lifecycle over the S3 API; the bucket therefore has no auto-expiry. This is harmless (sccache objects are regenerable and sccache never evicts its own S3 objects) but means the bucket grows slowly over time. Revisit retention via the object-store admin UI or a small cron only if it ever gets large.

## Workflow shape (`ci.yml`)

Workflow-level: `concurrency` group with `cancel-in-progress`, `permissions: contents: read`. Non-secret env (`CARGO_TERM_COLOR`, `RUST_BACKTRACE`) global. `RUSTFLAGS="-D warnings"` is set at the **job `env:`** level on the compiling jobs (`rust`, `python`) rather than workflow-global, so it never reaches the doc/lint jobs (`deny`, `docs`) or the host-side `e2e`/`ci-image` work.

### Trusted family (self-hosted + container)

Jobs `rust`, `deny`, `docs`, `python` (matrix 3.10 / 3.14 — the supported floor and ceiling):

- `runs-on: [self-hosted, Linux, platform-builder-docker-xl, platform-builder-Ubuntu-22.04]`
- `container: { image: eccr.ecmwf.int/aviso/cli-ci:<FULL_VERSION>, credentials: ... }` — **pinned full tag, never `:latest`**.
- `if: github.event_name != 'pull_request' || github.event.pull_request.head.repo.full_name == github.repository` (the `!= 'pull_request'` form so `push` and `workflow_dispatch` both run, and only PRs are origin-checked)

**sccache env scoped to the Rust-compiling jobs only** (`rust`, `python`; `deny`/`docs` never receive S3 creds):

```yaml
RUSTC_WRAPPER: sccache
CARGO_INCREMENTAL: "0"
SCCACHE_BUCKET: aviso-client-ci-cache
SCCACHE_ENDPOINT: "https://object-store.os-api.cci1.ecmwf.int"
SCCACHE_REGION: us-east-1
SCCACHE_S3_USE_SSL: "true"
SCCACHE_S3_KEY_PREFIX: aviso-client/
SCCACHE_BASEDIRS: ${{ github.workspace }}
SCCACHE_IGNORE_SERVER_IO_ERROR: "1"
AWS_ACCESS_KEY_ID: ${{ secrets.S3_ACCESS_KEY_ID }}
AWS_SECRET_ACCESS_KEY: ${{ secrets.S3_SECRET_ACCESS_KEY }}
TMPDIR: ${{ github.workspace }}/.tmp
```

Per-job shape: `mkdir -p "$TMPDIR"` -> the checks -> `sccache --show-adv-stats` (`if: always()`) -> `rm -rf "$TMPDIR" target .venv` (`if: always()`). Rely on sccache as the cross-run cache; clean `target` rather than persisting it (persistent `target` on shared disk invites stale-state, ownership, and branch cross-talk bugs; revisit only if speed is inadequate, and cache the cargo registry / uv cache before persistent `target`).

Job specifics:

- `deny`, `docs` call the baked `cargo-deny` / `mdbook` / `mdbook-mermaid` binaries directly (no install step).
- `python`: select the interpreter with `UV_PYTHON=${{ matrix.python }}` (never `uv python pin`, which mutates `.python-version`). Set `UV_PROJECT_ENVIRONMENT=${{ github.workspace }}/.venv` so cleanup is deterministic. Then `uv sync --locked --group dev`, `uv run maturin develop --locked`, ruff check / format --check, `ty check`, pytest.
- `rust`: fmt --check, clippy, build, test, doctest, `git diff --exit-code Cargo.lock`. If "not a git repository" appears in-container, add `git config --global --add safe.directory "$GITHUB_WORKSPACE"` (scoped, not `'*'`).

sccache reality check: it skips linker / `bin` / `cdylib` / `proc-macro` units, so the `aviso-cli` bin link and the `aviso-py` cdylib final link will not cache, but dependency `rlib`s (the bulk of build time) will.

### e2e-config

`docker compose -f tests/e2e/docker-compose.yml config --quiet` stays on GitHub-hosted `ubuntu-latest` (no compile, no secrets, awkward to nest docker in the CI container).

### ci-pass aggregator

GitHub-hosted `ubuntu-latest`, `needs:` every job above, `if: always()`, **fails if any needed job's result is not `success`** (so skipped trusted jobs on a fork PR -> red -> the fork PR cannot merge). This is the sole required status check in branch protection, giving one stable name despite the python matrix.

### e2e suite (real stack)

A trusted `e2e` job runs the full e2e suites against the real `aviso-server` + `auth-o-tron` + NATS compose stack. The host only orchestrates Docker (it already has docker/compose/curl); the suites build and run **inside the CI image joined to the compose network** via `docker run --network ${COMPOSE_PROJECT_NAME}_default`, reaching services by name (`AVISO_E2E_BASE_URL=http://aviso-server:8000`). This sidesteps two traps: the host has no Rust/uv/gcc, and a 24.04-container artifact will not run on the 22.04 host (glibc). Auth is `base_url`-independent (fixed JWT `iss`/`aud`), and the reconnect test confirmed the client reconnects to its own configured URL, so no server `base_url` override is needed.

Key mechanics: the container runs `--user $(id -u):$(id -g)` with `HOME`/`CARGO_HOME`/`CARGO_TARGET_DIR`/`SCCACHE_DIR`/`UV_*`/`XDG_CACHE_HOME` redirected to `/tmp`, so nothing root-owned lands in the workspace. `tests/e2e/python/conftest.py` honors a new `AVISO_E2E_EXTERNAL_STACK=1` flag to skip its own `stack.sh up` (no docker inside the runner container). Isolation: job-level `concurrency: { group: aviso-client-e2e, cancel-in-progress: false }` serializes e2e repo-wide against the fixed host ports / compose project; `down -v --remove-orphans` plus a labelled-container reap run before and after to survive a cancelled or hard-killed prior run. Hardening: `persist-credentials: false`, a per-run `DOCKER_CONFIG` under `RUNNER_TEMP` removed in `always()`, secrets step-scoped. **e2e is intentionally NOT in `ci-pass`** (informational) until a flake-free soak; promotion is just adding it to `ci-pass`'s `needs`. Validated end-to-end locally against the real eccr stack: Python suite + all five Rust e2e tests (including the 17.5s reconnect test) passed.

## Roll-out (separate PRs)

1. **Bootstrap image.** Add `Dockerfile` + `VERSION` (`0.1.0`) + `ci-image.yml`; merge and confirm `eccr.ecmwf.int/aviso/cli-ci:0.1.0` is pushed **before** anything consumes it. Run `create-cache-bucket.sh` once.
2. **Migrate `ci.yml`.** Rewrite into the trusted family + `e2e-config` + `ci-pass`, pinned to the bootstrap tag (`:0.1.0` at first; bumped to `:0.2.0` once `python3-dev` was baked, see Status). First real self-hosted run validates on this PR. Update branch protection to require `ci-pass`.
3. **Docs.** `CONTRIBUTING.md` (fork-PR policy: maintainers land external work on a repo branch for the gate to run), `README.md`, `tests/e2e/README.md` note.

## Open / to confirm at implementation

- Bucket region: assumed `us-east-1` (tensogram's value). Adjust the script + workflow env together if the object store wants a different one.
- First image VERSION: `0.1.0`.

## Status

- Planned and agreed. Bucket `aviso-client-ci-cache` provisioned (no lifecycle rule; see above).
- Branch `ci/self-hosted-sccache`: CI image built, validated, and **pushed to eccr** (`Dockerfile`, `VERSION` = `0.1.0`, `ci-image.yml`). Tags `0.1.0` / `0.1` / `0` / `latest` are live on digest `sha256:85290a37…`. The bootstrap push was done by hand from local (the consuming workflow can't pull an image that does not exist yet); `ci-image.yml` handles every subsequent rebuild.
- `ci.yml` rewritten onto the trusted family (`rust`, `deny`, `docs`, `python` 3.10/3.14 in the container; sccache scoped to `rust`/`python`) plus GitHub-hosted `e2e-config` and the `ci-pass` aggregate gate. `.github/actionlint.yaml` declares the self-hosted labels; both workflows pass actionlint.
- PR #24 opened. First run validated the self-hosted path: `deny`, `mdBook`, `e2e-config` passed; sccache worked (0 cache errors). The Rust job failed because the lean `0.1.0` image had no Python for the pyo3 workspace build. Fixed by baking `python3-dev` and bumping the image to `0.2.0` (pushed); `ci.yml` pins `:0.2.0`.
- Added the trusted `e2e` job (compose-network test-runner; see "e2e suite" above) plus the `AVISO_E2E_EXTERNAL_STACK` conftest flag. Two Oracle reviews (design + implementation) passed; validated end-to-end locally against the real eccr stack (Python + all Rust e2e tests including reconnect). e2e is informational (not in `ci-pass`) pending a soak.
- Next action: confirm the green run on PR #24 (first real sccache-S3 e2e run), watch sccache startup/object-store auth + any leaked runner containers, then set branch protection to require `ci-pass`.
