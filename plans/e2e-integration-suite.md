# End-to-end integration test suite

Replace the hermetic-only test posture with a real-services integration suite that exercises the supervisor, state store, auth refresh, trigger dispatcher, and CLI subcommands against actual `aviso-server`, `auth-o-tron`, and NATS containers. Hermetic tests stay for fast feedback; the new suite catches the protocol and behaviour bugs the hermetic ones cannot.

## Motivation

The merged client suite (Rust core + CLI + Python bindings) is exercised today by:

- 400+ Rust unit tests using `wiremock` for HTTP and synthetic SSE streams.
- 120 Python tests that verify binding shape, value types, stub completeness, property tests on filters.
- A local doc-examples harness that runs every runnable Python code block against `aviso-server.ecmwf.int` (the ECMWF dev server).

Three behaviour classes are not covered by any of these:

1. **Real-wire reconnect on routine `max_duration_reached`**: wiremock cannot fake the `connection-closing` event the real server emits every `connection_max_duration_sec`, and the dev server's 1-hour rotation is too slow for tests.
2. **Auth refresh on real 401**: wiremock cannot rotate tokens against a real auth backend.
3. **CLI subcommands against a real server**: every CLI subcommand (`notify`, `listen`, `replay`, `schema list/get`, `admin wipe-*`, `config dump`) has only the wiremock surface from Phase 5.

The infrastructure to fix this exists. `tests/e2e/docker-compose.yml` ships an `aviso-server` pinned by digest with a 60-second `connection_max_duration_sec` and a 5-second heartbeat (`tests/e2e/aviso-server.config.yaml`), which are the right knobs to surface reconnect bugs fast. The README explicitly notes: "The full e2e suite runs once real tests exist." This plan delivers that suite.

## What ships

### Compose stack additions

The existing `tests/e2e/docker-compose.yml` gains two services and the `aviso-server` config moves from in-memory + no-auth to NATS-backed + auth-o-tron-authenticated:

- **`nats`**: JetStream-enabled NATS server. Persistent volume per stack instance so a `docker compose restart` does not wipe history. Exposed on `127.0.0.1:${NATS_HOST_PORT:-4222}`.
- **`auth-o-tron`**: ECMWF's auth service. Config carries three test accounts:
  - `tester` / `tester-token` (long-lived bearer, never expires) for the happy-path tests.
  - `rotating` / `rotating-token-v1` (initial bearer, gets rotated to `rotating-token-v2` mid-test) for the 401-refresh test.
  - `readonly` / `readonly-token` (no publish permission) for the 403-permission test.

`aviso-server.config.yaml` flips to `notification_backend.kind: nats` and `auth.enabled: true` with the auth-o-tron URL. The in-memory + no-auth config moves to `aviso-server.in-memory.config.yaml` and a second compose service `aviso-server-in-memory` exposes it on a different port for the fast-feedback variant of the same tests.

The compose file gains a `nats.conf` and an `auth-o-tron.config.yaml` mounted into their respective containers.

### Test harness layout

```
tests/e2e/
├── docker-compose.yml              (extended)
├── aviso-server.config.yaml        (NATS backend, auth enabled)
├── aviso-server.in-memory.config.yaml  (NEW: fast-feedback variant, in-memory + no-auth)
├── auth-o-tron.config.yaml         (NEW: 3 test accounts)
├── nats.conf                       (NEW: JetStream enabled, basic limits)
├── README.md                       (extended)
├── shared/
│   └── stack_up.sh                 (NEW: shell helper both python and rust harnesses call)
├── python/                         (NEW)
│   ├── conftest.py                 (stack-up fixture per pytest-xdist worker)
│   ├── test_publish_listen.py
│   ├── test_reconnect.py
│   ├── test_auth_refresh.py
│   ├── test_resume_across_restart.py
│   ├── test_history_gap.py
│   ├── test_triggers_real_dispatch.py
│   ├── test_multiplex_async.py
│   └── test_flush_cursor_on_exit.py
└── rust/                           (NEW)
    ├── Cargo.toml                  (test crate, aviso = { workspace = true })
    ├── src/
    │   └── lib.rs                  (shared stack-up helper for cargo tests)
    └── tests/
        ├── library_publish_listen.rs
        ├── library_reconnect.rs
        ├── library_auth_refresh.rs
        ├── cli_publish.rs
        ├── cli_listen_yaml.rs
        └── cli_replay_history.rs
```

The Python suite is the primary user-facing validator. The Rust suite covers library API ergonomics and CLI behaviour that the Python suite cannot reach.

### Test scenarios

#### Python suite (`tests/e2e/python/`)

10 scenarios targeting the user-facing API:

| File | What it asserts |
|---|---|
| `test_publish_listen.py` | Publish N notifications, listen for them, assert all N received in order with matching identifier and payload. |
| `test_reconnect.py` | Open a listener, let `connection_max_duration_sec` fire (60s), count notifications across the cut. Asserts at-least-once with at-most-one duplicate per cut. Also covers `docker compose restart aviso-server` mid-stream. |
| `test_auth_refresh.py` | Use the `rotating` account with `auth.Bearer("rotating-token-v1")`, force auth-o-tron to expire the token mid-stream, assert the supervisor's `refresh()`-then-retry path delivers no user-visible error. |
| `test_resume_across_restart.py` | Listen with `JsonFileStore`, kill the Python process after N notifications, restart, assert the next pull is `N+1` with no replay and no gap. |
| `test_history_gap.py` | Fill NATS past the configured retention, request replay from a pruned sequence, assert `HistoryGapError(reason="replay_limit_reached")` with `.max_allowed` populated. |
| `test_triggers_real_dispatch.py` | Echo + log triggers dispatched server-side; assert stdout and file contents match expected. Webhook trigger pointed at an in-process `http.server.HTTPServer`. |
| `test_multiplex_async.py` | Two `AsyncAvisoClient.listen()` calls under `asyncio.gather`; assert per-stream ordering preserved, cross-stream interleaving is non-deterministic. |
| `test_flush_cursor_on_exit.py` | `flush_cursor_on_exit=True` + iterator `with`; clean shutdown, then restart; assert no replay of the last notification. |
| `test_schema_discovery.py` | `client.schema()` + `client.schema_for(...)` against the test stack; assert returned shape matches the configured schemas. |
| `test_concurrent_publishers.py` | Five `asyncio.gather`'d publishes; assert all five `request_id`s are unique and all five appear on a subsequent listen. |

#### Rust suite (`tests/e2e/rust/`)

6 scenarios targeting Rust-specific surfaces:

| File | What it asserts |
|---|---|
| `library_publish_listen.rs` | `AvisoClient::builder()` + `client.notify(...).await?` + `client.watch(req)?` against the stack. Validates the Rust API ergonomics, not just the binding. |
| `library_reconnect.rs` | Same reconnect scenario as the Python test, from the Rust library caller's POV. Catches Rust-API-specific bugs (lifetimes, error propagation, supervisor lifetime) that the binding might hide. |
| `library_auth_refresh.rs` | Custom `AuthProvider` impl with a refresh hook; rotate token via auth-o-tron; assert the trait contract is honoured. |
| `cli_publish.rs` | `aviso notify` subcommand: argument parsing, `--config` resolution, identifier flag parsing, stdout shape. |
| `cli_listen_yaml.rs` | `aviso listen --config <yaml>` with a listener YAML file that exercises every trigger kind; assert stdout / file outputs / process exit code. |
| `cli_replay_history.rs` | `aviso replay --from <N>` against a stream that has been pushed past N; assert ordering and end-of-stream exit. |

### CI integration

A new GitHub Actions job `e2e` runs on the existing CI workflow:

- **Runs on**: push to `main` and on PRs that touch `crates/`, `python/`, `tests/e2e/`, or the workflow file itself.
- **Skipped on**: pure docs PRs (paths-filter on `crates/`, `python/`, `tests/e2e/`).
- **Required for merge** to `main`; informational on in-flight PRs.
- **Runner**: GitHub-hosted `ubuntu-latest`.
- **Image registry**: public; no `docker login` step needed.
- **Steps**:
  1. Checkout.
  2. Set up Rust toolchain (cached), uv (cached).
  3. `docker compose -f tests/e2e/docker-compose.yml pull` (parallel pulls all three images).
  4. Build the aviso Python extension (`uv run maturin develop --locked`) and the Rust CLI (`cargo build --release -p aviso-cli`).
  5. Bring up the stack: `docker compose -f tests/e2e/docker-compose.yml up -d --wait`.
  6. Run Python suite: `uv run pytest tests/e2e/python/ -v`.
  7. Run Rust suite: `cargo test -p aviso-e2e --test '*'` (the new `tests/e2e/rust/` is its own workspace member).
  8. Tear down: `docker compose -f tests/e2e/docker-compose.yml down -v`.
- **Failure surface**: on test failure, upload `docker compose logs` as an artifact for post-mortem.
- **Resource budget**: ~3-5 minutes added to the CI cycle. Acceptable.

### What this changes about the existing test surface

- **Hermetic Python tests stay** (`python/tests/`). Fast feedback loop (~2s) remains. These exercise the API shape; the e2e suite exercises the behaviour.
- **Hermetic Rust tests stay** (`crates/aviso/tests/`, `crates/aviso-cli/tests/`). The supervisor's classifier tests, the SSE parser tests, the wiremock-driven supervisor stress tests all stay. The new Rust e2e suite is additive, not a replacement.
- **The doc-examples harness** moves to optionally point at the local stack via `AVISO_BASE_URL=http://localhost:8000` (the local default). It can also keep pointing at `aviso-server.ecmwf.int` for the maintainer's local validation; the harness becomes server-agnostic via the env var.
- **Currently-`<!-- not-runnable -->`-marked snippets** become runnable against the local stack (the webhook example with placeholder URL, the command example with a custom shell command, the resume example with a persistent state file). The harness's not-runnable list shrinks.

## Decisions

### D-S1. Same compose stack serves both language suites

One compose file, one stack-up call, both languages run against it sequentially (or in parallel if resource pressure allows). Two stacks would be wasteful and would introduce cross-stack drift.

### D-S2. One stack per pytest-xdist worker

The existing pattern in `tests/e2e/README.md` (`AVISO_SERVER_HOST_PORT=8101 docker compose -p shard-1 up -d`) supports this. Each pytest worker gets its own stack so parallel test runs do not interfere. The `conftest.py` fixture allocates a port per worker (using `os.environ["PYTEST_XDIST_WORKER"]` to derive a stable offset) and brings up a fresh stack at session scope.

### D-S3. JetStream-backed NATS, not in-memory NATS

In-memory NATS would skip persistence semantics that real users care about (resume across NATS restart, retention pruning). JetStream is the only option that exercises the full backend. A separate `aviso-server-in-memory` service stays in the compose file for fast-feedback / hermetic-ish tests.

### D-S4. Real auth-o-tron, not a mock auth server

Mocking auth would test the binding's auth-provider interface but not the real refresh-on-401 flow. auth-o-tron is the canonical auth service the production aviso-server users; testing against it validates the contract.

### D-S5. Image versions pinned by digest

The existing `aviso-server` is pinned `0.6.2@sha256:...`. The new `nats` and `auth-o-tron` services follow the same convention: tag for human readability, digest for reproducibility. README documents the bump procedure.

### D-S6. Rust e2e suite is its own workspace member

`tests/e2e/rust/Cargo.toml` declares an `aviso-e2e` package. The workspace `Cargo.toml` adds it as a member. Tests live under `tests/e2e/rust/tests/` and run via `cargo test -p aviso-e2e --test '*'`. This keeps the e2e tests out of `cargo test --workspace` (which CI runs on every PR) and into a deliberate `e2e` job.

### D-S7. Stack-up is fast enough to not need session-scope sharing across pytest files

`docker compose up -d --wait` for the three services takes ~5-10 seconds on a warm runner (images cached). The fixture is session-scoped so each test file does not re-pay the cost. If tests cross-contaminate state (a test publishes notifications that another test sees), we use unique `event_type` values per test, or wipe between tests via the admin API.

### D-S8. Failure logs are uploaded as a CI artifact

On any e2e test failure, the workflow runs `docker compose logs > e2e-logs.txt` and uploads it via `actions/upload-artifact`. Without this, debugging an e2e failure means re-running locally with the same images and hoping the bug reproduces.

## Open questions

1. **Image references**. The user said both ECMWF images are public; the exact tag and digest to pin will come from the user in the implementation phase. The plan reserves slots for `eccr.ecmwf.int/aviso/auth_o_tron:<TAG>@<DIGEST>` and `docker.io/library/nats:<TAG>@<DIGEST>` (or wherever the user names).
2. **auth-o-tron config schema**. Need to read auth-o-tron's documentation or source to understand the config format for the three test accounts. Will be settled during implementation.
3. **NATS retention limits**. The `test_history_gap.py` test needs a deterministic way to force pruning. Either set a small `max_msgs` in `nats.conf` and publish past it, or use the JetStream admin API to delete oldest. To be settled during implementation.
4. **Test parallelism budget**. GitHub-hosted runner has 2 cores and 7 GB RAM. Three services running plus a pytest-xdist worker leaves ~5 GB headroom. Single-worker is fine for the initial suite; parallelism can be added later if test count grows.

## Roll-out

Five focused commits on `feat/e2e-integration-suite`:

1. **Commit 1: compose stack + configs**. Extend `docker-compose.yml` with `auth-o-tron` and `nats` services; add their config files; flip `aviso-server.config.yaml` to NATS backend + auth enabled; add the `aviso-server.in-memory` service for the fast-feedback variant; update `tests/e2e/README.md` to document the new shape and the bump procedure for the new pinned images.

2. **Commit 2: stack-up helper + Python conftest.py**. Add `shared/stack_up.sh` (a shell helper both languages call to bring the stack up against a chosen port shard) and `python/conftest.py` (session-scoped fixture, per-xdist-worker shard allocation). No tests yet; just the scaffolding.

3. **Commit 3: Python e2e suite**. All 10 Python test files; runs against the stack from Commit 2.

4. **Commit 4: Rust e2e suite**. New `tests/e2e/rust/Cargo.toml` workspace member, the 6 Rust test files, workspace `Cargo.toml` updated.

5. **Commit 5: CI workflow + final docs**. New `.github/workflows/ci.yml` job (or extends the existing one) running the e2e suite per D-S2 + D-S8; updates `CONTRIBUTING.md` and `README.md` to document local-run workflow.

## What stays the same

- All current hermetic tests stay (Rust + Python).
- The doc-examples harness stays (the maintainer's local fact-check tool); only its server target becomes configurable.
- The public Python API surface from PR #21 is unchanged.
- The Rust core, CLI, and bindings are not touched by this work; only configuration, harness, and tests are added.

## What changes for users

Nothing user-facing. This is internal test infrastructure. The only externally observable difference is "CI is more confident about behaviour changes": same green/red signal, more meaningful.

## Status snapshot

- Branch: `feat/e2e-integration-suite` (created off `main` at `5f16885`)
- Plan: this file
- Not yet implemented
- Image references: TBD (user to supply during implementation phase)
- Awaiting: oracle plan review, then user go-ahead for execution
