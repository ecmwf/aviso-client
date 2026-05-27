# End-to-end integration test suite

Replace the hermetic-only test posture with a real-services integration suite that exercises the supervisor, state store, auth refresh, trigger dispatcher, and CLI subcommands against actual `aviso-server`, `auth-o-tron`, and NATS containers. Hermetic tests stay for fast feedback; the new suite catches the protocol and behaviour bugs the hermetic ones cannot.

## Motivation

The merged client suite (Rust core + CLI + Python bindings) is exercised today by:

- 400+ Rust unit tests using `wiremock` for HTTP and synthetic SSE streams.
- 120 Python tests that verify binding shape, value types, stub completeness, property tests on filters.
- A local doc-examples harness that runs every runnable Python code block against `aviso-server.ecmwf.int` (the ECMWF dev server).

Three behaviour classes are not covered by any of these:

1. **Real-wire reconnect on routine `max_duration_reached`**: wiremock cannot fake the `connection-closing` event the real server emits every `connection_max_duration_sec`, and the dev server's 1-hour rotation is too slow for tests.
2. **Auth refresh on real 401**: wiremock cannot rotate credentials against a real auth backend.
3. **CLI subcommands against a real server**: every CLI subcommand (`notify`, `listen`, `replay`, `schema list/get`, `admin wipe-*`, `config dump`) has only the wiremock surface from earlier work.

The infrastructure to fix this exists. `tests/e2e/docker-compose.yml` ships an `aviso-server` pinned by digest with a 60-second `connection_max_duration_sec` and a 5-second heartbeat (`tests/e2e/aviso-server.config.yaml`), which are the right knobs to surface reconnect bugs fast. The README explicitly notes: "The full e2e suite runs once real tests exist." This plan delivers that suite.

## What ships

### Compose stack additions

The existing `tests/e2e/docker-compose.yml` gains two services and the `aviso-server` config moves from in-memory + no-auth to NATS-backed + auth-o-tron-authenticated:

- **`nats`**: JetStream-enabled NATS server. Persistent volume per stack instance so a `docker compose restart` does not wipe history. Exposed on `127.0.0.1:${NATS_HOST_PORT:-4222}`. A small `max_msgs` is configured at the stream level so `test_history_gap.py` can deterministically force pruning by publishing past the limit (count-based retention, not time-based).
- **`auth-o-tron`**: ECMWF's auth service. The aviso-server runs in `direct` auth mode (forwards Basic credentials to auth-o-tron for validation) per D8. Three test accounts:
  - `tester` / `tester-token`: long-lived credential for happy-path tests.
  - `rotating` / `rotating-token-v1` plus a second valid credential `rotating-token-v2`: both live in the auth-o-tron config from the start. The refresh test rewrites a sidecar file the client reads via `ConfigFileAuth` from `v1` to `v2` mid-stream; auth-o-tron itself is never restarted.
  - `readonly` / `readonly-token`: account with no publish permission, for the 403 test.

`aviso-server.config.yaml` flips to `notification_backend.kind: nats` (pointed at the `nats` service), `auth.enabled: true` with the auth-o-tron URL, and `connection_max_duration_sec: 15` (down from 60). The shorter cycle keeps reconnect tests deterministic without dominating the CI floor: 15 seconds is long enough that a publish-and-listen pair completes inside one connection (no false cuts in non-reconnect tests) and short enough that `test_reconnect.py` pays ~20 s wall-time instead of ~70. The 60 s value the PR #21 docs used was a maintainer-comfort knob for manual `docker compose logs -f` debugging; 15 s is the right value for an automated suite. The `notification_schema` block (with `test_event` and `test_polygon`) is preserved verbatim; the schema snippet that PR #21 added to `docs/src/python/quickstart.md` continues to source from the same file.

The compose file gains a `nats.conf` and an `auth-o-tron.config.yaml` mounted into their respective containers.

### Test harness layout

```text
tests/e2e/
├── docker-compose.yml              (extended: + nats + auth-o-tron)
├── aviso-server.config.yaml        (NATS backend, auth enabled)
├── auth-o-tron.config.yaml         (NEW: 3 test accounts, direct mode)
├── nats.conf                       (NEW: JetStream enabled, small max_msgs)
├── README.md                       (extended)
├── shared/
│   └── stack_up.sh                 (NEW: brings stack up + polls readiness)
├── python/                         (NEW)
│   ├── conftest.py                 (session-scoped stack fixture; single shared stack)
│   ├── test_publish_listen.py
│   ├── test_reconnect.py
│   ├── test_auth_refresh.py
│   ├── test_resume_across_restart.py
│   ├── test_history_gap.py
│   ├── test_triggers_real_dispatch.py
│   ├── test_multiplex_async.py
│   ├── test_flush_cursor_on_exit.py
│   ├── test_schema_discovery.py
│   └── test_concurrent_publishers.py
└── rust/                           (NEW)
    ├── Cargo.toml                  (workspace member; tests `#[ignore]`-gated)
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

Ten scenarios targeting the user-facing API. Tests use the PR #21 API surfaces: `client.listen(..., triggers=[...])` with the `triggers=` kwarg, and `with client.listen(...) as iterator:` / `async with client.listen(...) as iterator:` for clean shutdown. Identifier-space isolation per D-S7 (unique polygons or `(date, time)` pairs per test).

| File | What it asserts |
|---|---|
| `test_publish_listen.py` | Publish N notifications, listen for them via `with client.listen(...) as iterator:`, assert all N received in order with matching identifier and payload. |
| `test_reconnect.py` | Open a listener, let `connection_max_duration_sec` fire (15s per the compose-config tweak), count notifications across the cut. Asserts at-least-once with at-most-one duplicate per cut. Also covers `docker compose restart aviso-server` mid-stream. |
| `test_auth_refresh.py` | Use the `rotating` account via `ConfigFileAuth` reading from a sidecar credentials file. Rewrite the file from `rotating-token-v1` to `rotating-token-v2` mid-stream, then force the connection cut. The next reconnect's POST goes through auth-o-tron with the stale token, returns 401, the supervisor's `refresh()` hook re-reads the file (D8), retry succeeds, no user-visible error surfaces on the iterator. |
| `test_resume_across_restart.py` | Listen with `JsonFileStore`, kill the Python process after N notifications, restart, assert the next pull is `N+1` with no replay and no gap. |
| `test_history_gap.py` | Publish past the configured NATS `max_msgs` limit so the stream prunes the oldest, request replay from a known pruned sequence, assert `HistoryGapError(reason="replay_limit_reached")` with `.max_allowed` populated. |
| `test_triggers_real_dispatch.py` | `client.listen(..., triggers=[Trigger.echo(), Trigger.log(path), Trigger.webhook(url)])` with the local webhook pointed at an in-process `pytest-httpserver`. Assert stdout, file contents, and webhook receipts all match expected. |
| `test_multiplex_async.py` | Two `async with client.listen(...) as iterator:` calls under `asyncio.gather`; assert per-stream ordering preserved, cross-stream interleaving is non-deterministic. |
| `test_flush_cursor_on_exit.py` | `flush_cursor_on_exit=True` + iterator `with` block; clean shutdown, then restart; assert no replay of the last notification. |
| `test_schema_discovery.py` | `client.schema()` + `client.schema_for(...)` against the test stack; assert returned shape matches the configured schemas. |
| `test_concurrent_publishers.py` | Five `asyncio.gather`'d publishes; assert all five `request_id`s are unique and all five appear on a subsequent listen. |

#### Rust suite (`tests/e2e/rust/`)

Six scenarios targeting Rust-specific surfaces. Every `#[test]` function carries `#[ignore = "requires e2e compose stack"]` so workspace-wide `cargo test` invocations compile the crate but skip running these tests; developers (and the future CI per `plans/e2e-ci-self-hosted.md`) opt in via `cargo test -p aviso-e2e -- --include-ignored`.

| File | What it asserts |
|---|---|
| `library_publish_listen.rs` | `AvisoClient::builder()` + `client.notify(...).await?` + `client.watch(req)?` against the stack. Validates the Rust API ergonomics, not just the binding. |
| `library_reconnect.rs` | Same reconnect scenario as the Python test, from the Rust library caller's POV. Catches Rust-API-specific bugs (lifetimes, error propagation, supervisor lifetime) that the binding might hide. |
| `library_auth_refresh.rs` | `ConfigFileAuth` reading from a sidecar credentials file, mid-stream file rewrite, assert the trait contract is honoured from Rust. |
| `cli_publish.rs` | `aviso notify` subcommand: argument parsing, `--config` resolution, identifier flag parsing, stdout shape. |
| `cli_listen_yaml.rs` | `aviso listen --config <yaml>` with a listener YAML file that exercises every trigger kind; assert stdout / file outputs / process exit code. |
| `cli_replay_history.rs` | `aviso replay --from <N>` against a stream that has been pushed past N; assert ordering and end-of-stream exit. |

### CI integration (deferred)

CI integration is OUT OF SCOPE for this branch and is deferred to a separate plan: `plans/e2e-ci-self-hosted.md`. The reason: a GitHub-hosted `ubuntu-latest` runner would add ~3-5 min warm / ~6-9 min cold per PR (docker image pulls, cold Rust builds, the reconnect-test floor). The right answer is a self-hosted runner with persistent docker image cache plus `sccache` against the user's existing S3 bucket, getting the warm cycle to ~1-2 min. That work is blocked on assigning self-hosted runners to the repo, which the user is taking offline with a colleague.

What stays in CI in this branch:

- The existing `e2e-config` job (`docker compose -f tests/e2e/docker-compose.yml config --quiet`) keeps validating the docker-compose file's syntactic correctness. It runs on every PR and must stay green after Commit 1's compose-stack changes land.
- Nothing else CI-related changes. The e2e suite (Python + Rust) is locally runnable but not gated.

What this branch DOES ship for local use:

- The compose stack, the readiness-polling helper (`tests/e2e/shared/stack_up.sh`), and a documented one-command flow: `bash tests/e2e/shared/stack_up.sh && uv run pytest tests/e2e/python/ && cargo test -p aviso-e2e -- --include-ignored`.
- The Rust e2e tests are `#[ignore]`-gated per D-S6 so the existing `cargo test --workspace --all-targets` CI gate keeps passing without the stack up. Developers explicitly opt in via `--include-ignored`.
- The Python e2e tests live under `tests/e2e/python/` and are NOT picked up by the hermetic `pyproject.toml`'s `[tool.pytest.ini_options] testpaths = ["python/tests"]`. Developers run them explicitly via `uv run pytest tests/e2e/python/`.

### What this changes about the existing test surface

- **Hermetic Python tests stay** (`python/tests/`). Fast feedback loop (~2s) remains. These exercise the API shape; the e2e suite exercises the behaviour. `pyproject.toml`'s `[tool.pytest.ini_options] testpaths = ["python/tests"]` stays narrow so the e2e directory is never picked up by the hermetic run.
- **Hermetic Rust tests stay** (`crates/aviso/tests/`, `crates/aviso-cli/tests/`). The supervisor's classifier tests, the SSE parser tests, the wiremock-driven supervisor stress tests all stay. The new Rust e2e suite is additive, not a replacement.
- **The doc-examples harness** (the maintainer's `/tmp/aviso-py-fact-check/run_doc_examples.py` script that is not part of CI) becomes server-agnostic via `AVISO_BASE_URL`; after the stack lands the harness can point at `http://localhost:8000` instead of `aviso-server.ecmwf.int`.
- **The PR #21 schema snippet in `docs/src/python/quickstart.md`** continues to source from `tests/e2e/aviso-server.config.yaml`. The file's path does not change; only its `notification_backend` and `auth` blocks change. The `notification_schema:` subtree the snippet is sourced from stays verbatim.

### What this changes for the python/examples/ tree (D-S9)

This is one of the highest-leverage downstream wins of the e2e work and the entire reason we are doing it from a user-experience angle.

Today, the 15 example scripts under `python/examples/` assume the reader's server has the `test_polygon` event type configured. We document a fallback ("substitute your own event type if not") and ship a copy-paste schema snippet for operators (per the quickstart's "What is on your server" section + the existing `tests/e2e/aviso-server.config.yaml`), but a new user without an operator account on any aviso-server still has friction: they have to find a server, get credentials, hope it has `test_polygon`, and only then can they try the examples.

Once the e2e stack ships, that friction collapses to four commands run from the repo root:

```bash
docker compose -f tests/e2e/docker-compose.yml up -d
bash tests/e2e/shared/stack_up.sh
export AVISO_BASE_URL=http://localhost:8000 AVISO_USERNAME=tester AVISO_PASSWORD=tester-token
python python/examples/basics/01_publish.py
```

The `test_polygon` schema is already in `tests/e2e/aviso-server.config.yaml`, so every example runs out of the box against the local stack with no operator involvement.

**The examples README update and the quickstart rewrite are explicitly OUT OF SCOPE for this branch.** They land in a separate follow-up PR after the e2e PR merges. This boundary keeps the e2e PR focused on the stack + the test suite + the CI plumbing, and lets the user verify the stack works end-to-end before committing to the examples-rewrite shape. The follow-up will:

- Add a "Run the examples against the local stack" subsection to `python/examples/README.md` showing the commands above.
- Add a one-line cross-reference from `docs/src/python/quickstart.md`'s "What is on your server" section so the schema-snippet path frames itself as "two options: spin up the local stack, or paste this into your own server's config".
- Optionally promote `triggers/05_webhook.py` to runnable now that the harness has a local HTTP receiver pattern in `advanced/03_webhook_with_local_server.py`; reconsider whether the split-into-two-files pedagogy still makes sense once both are runnable.

The follow-up's scope is small (a docs edit and possibly the webhook-pair reconsideration); tracked here so it does not get lost.

## Decisions

### D-S1. Same compose stack serves both language suites

One compose file, one stack-up call, both languages run against it sequentially in a local or future-CI run. Two stacks would be wasteful and would introduce cross-stack drift.

### D-S2. Single session-scoped stack per pytest session

`tests/e2e/python/conftest.py` brings the stack up once via `bash tests/e2e/shared/stack_up.sh` at session scope, holds it for every test in the session, and tears it down after the last test. No pytest-xdist sharding in the initial implementation: the ten Python scenarios run sequentially with the reconnect-test floor at ~20 s, comfortably within a single local run, and adding xdist would require port-sharding logic plus the `pytest-xdist` dev dependency for no concrete payoff. The existing `tests/e2e/README.md` parallel-shard pattern (`AVISO_SERVER_HOST_PORT=8101 -p shard-1`) survives as documentation for human operators who want to run the suite locally alongside other instances; the test fixture itself does not exercise it. If a future test count growth makes the suite slow enough to warrant parallelism, xdist + per-worker port sharding lands as a small follow-up.

### D-S3. JetStream-backed NATS, no separate in-memory variant

In-memory NATS would skip persistence semantics that real users care about (resume across NATS restart, retention pruning). JetStream is the only option that exercises the full backend.

The plan originally proposed a second `aviso-server-in-memory` service for "fast-feedback / hermetic-ish" runs. That is dropped from the initial implementation. The existing hermetic Python (`python/tests/`) and Rust (`crates/*/tests/`) tests already provide the sub-second feedback loop, and the cost of a second aviso-server service (a second config file to maintain, twice the per-test stack-up time when the suite scales out) outweighs the benefit when no named scenario requires the in-memory variant. If a future scenario does need it (a fast-iteration test that intentionally bypasses NATS persistence), it lands as a small follow-up.

### D-S4. Real auth-o-tron in direct mode; refresh test driven by a file rewrite

Mocking auth would test the binding's auth-provider interface but not the real refresh-on-401 flow. auth-o-tron is the canonical auth service the production aviso-server uses; testing against it validates the contract.

aviso-server runs in `direct` auth mode (forwards Basic credentials to auth-o-tron for validation) per D8. The refresh test (`test_auth_refresh.py`) uses `ConfigFileAuth` reading from a sidecar credentials file under the pytest tmpdir. The auth-o-tron config holds both credentials (`rotating-token-v1` and `rotating-token-v2`) as valid from session start, with the same `rotating` user. Mid-stream the test rewrites the sidecar file from one to the other and forces the connection cut (either by waiting for `connection_max_duration_sec` to fire or by restarting aviso-server). The next reconnect's POST goes through auth-o-tron with the stale token, returns 401, the supervisor's `refresh()` hook re-reads the file per D8's refresh-then-retry-once semantics, and the retry succeeds with no user-visible error on the iterator.

auth-o-tron itself is never restarted; only the client-side credentials file is rewritten. This avoids any race against auth-o-tron's startup time and keeps the refresh mechanism purely client-side.

### D-S5. Image versions pinned by digest

The existing `aviso-server` is pinned `0.6.2@sha256:...`. The new `nats` and `auth-o-tron` services follow the same convention: tag for human readability, digest for reproducibility. README documents the bump procedure.

### D-S6. Rust e2e suite is a workspace member with `#[ignore]`-gated tests

`tests/e2e/rust/Cargo.toml` declares an `aviso-e2e` package. The workspace `Cargo.toml` adds it as a member so `cargo build --workspace --all-targets`, `cargo clippy --workspace --all-targets`, and `cargo test --workspace --all-targets` (the gates the existing CI Rust job runs on every PR) all compile the e2e crate's test code and catch type errors. Every `#[test]` function in the crate carries `#[ignore = "requires e2e compose stack"]`, so the normal `cargo test --workspace` invocation skips actually running them. The dedicated e2e job opts in via `cargo test --locked -p aviso-e2e -- --include-ignored`.

The alternative (a separate top-level Cargo project outside the workspace) was considered and rejected because workspace membership keeps the dev-dep deduplication and the `aviso = { workspace = true }` import shape; the per-test `#[ignore]` annotation costs nothing in maintenance and gives a clear "why this is skipped" signal in `cargo test` output.

`cargo deny check` already covers any new dev-dependencies the e2e crate brings; no separate deny.toml entry is required.

### D-S7. Tests isolate via disjoint identifier values, not unique event types

The stack ships two event-type schemas in `tests/e2e/aviso-server.config.yaml` (`test_event` and `test_polygon`); inventing a per-test event type would require either runtime schema registration (not supported by aviso-server) or extra `notification_schema` entries in the test config (maintenance overhead for no payoff). Instead, each test reserves a disjoint slice of the identifier space and filters its watch to that slice:

- For `test_polygon`, each test uses a unique polygon string (the polygon serves as a discriminator that the filter can pin; the schema declares polygon `required: true` so it appears in every notify and every watch filter).
- For `test_event`, each test uses a unique `(date, time)` pair.

The admin wipe endpoint is reserved for tests that intentionally mutate global server state as the test under test (e.g., a test for `aviso admin wipe-stream` itself); routine isolation does not need it. This keeps the suite order-independent and parallelisable later without per-test cleanup machinery.

The stack-up cost is paid once per session: `bash tests/e2e/shared/stack_up.sh` brings up all three services and polls for readiness, then the session-scoped fixture in `tests/e2e/python/conftest.py` holds the stack for every test in the session.

### D-S8. Failure logs are captured for local debugging

On any e2e test failure during a local run, the test harnesses (`tests/e2e/python/conftest.py` for the Python suite, the helper in `tests/e2e/rust/src/lib.rs` for the Rust suite) capture `docker compose -f tests/e2e/docker-compose.yml logs > tests/e2e/last-failure.log` in a session-scope teardown that runs only on failure. The same capture pattern is reused by the future CI workflow (`plans/e2e-ci-self-hosted.md`) which uploads the file as an artifact; for now the file is local-only.

### D-S9. The same compose stack is the recommended local environment for running the `python/examples/` tree

The e2e suite is the primary consumer of the stack, but the stack is reusable for any local validation. Specifically: the `python/examples/` scripts assume a server with `test_polygon` configured; the e2e stack ships that schema in `tests/e2e/aviso-server.config.yaml`. A new user evaluating the Python API will be told to run the same `docker compose up -d` that the test suite uses, point `AVISO_BASE_URL` at `http://localhost:8000`, and run any example unchanged. This collapses the "find a server, get credentials, hope it has test_polygon" friction to four commands.

**The follow-up examples PR documents this; the e2e work in THIS branch does not touch `python/examples/README.md` or `docs/src/python/quickstart.md`.** The e2e branch only ships the stack + the test suite + the CI plumbing. The examples-and-quickstart rewrite is a separate, smaller PR after the e2e PR merges. Keeping the two PRs separate lets the user verify the stack works end-to-end before committing to the examples-rewrite shape, and keeps the e2e PR focused on one concern per AGENTS.md.

## Open questions

1. **Image references**. The user has confirmed all three required images are public; the exact tag and digest for `auth-o-tron` and `nats` come from the user during Commit 1 implementation. The plan reserves slots for `eccr.ecmwf.int/aviso/auth_o_tron:<TAG>@<DIGEST>` and `docker.io/library/nats:<TAG>@<DIGEST>` (or wherever the user names). Pin format matches the existing `aviso_server` pattern (`<tag>@<digest>` on the `image:` line).

2. **auth-o-tron config field names**. The plan pins direct auth mode and three test accounts (`tester`, `rotating`, `readonly`); the exact YAML field names auth-o-tron uses for `users` / `tokens` / `roles` need to be read from auth-o-tron's source or docs during Commit 1 implementation. The decisions above (mode, accounts, refresh mechanism) are pinned; only the YAML key-spelling is implementation discovery.

## Roll-out

Four focused commits on `feat/e2e-integration-suite`. The branch is rebased onto current `main` (`a0096d3`) before Commit 1 lands; see Status snapshot.

1. **Commit 1: compose stack + configs**. Extend `docker-compose.yml` with `auth-o-tron` and `nats` services pinned by `<tag>@<digest>`; add `auth-o-tron.config.yaml` with three test accounts in direct mode per D-S4; add `nats.conf` with a small JetStream `max_msgs` for `test_history_gap` per the deterministic-pruning rule; flip `aviso-server.config.yaml` to `notification_backend.kind: nats`, `auth.enabled: true`, and `connection_max_duration_sec: 15` (down from 60) while preserving the `notification_schema` block verbatim. Update `tests/e2e/README.md` to document the new shape, the bump procedure for the new pinned images, the local-run workflow (`bash tests/e2e/shared/stack_up.sh && uv run pytest tests/e2e/python/`), and the credentials reference. The existing `e2e-config` CI gate (`docker compose -f tests/e2e/docker-compose.yml config --quiet`) must pass against the new shape.

2. **Commit 2: stack-up helper + Python conftest.py**. Add `tests/e2e/shared/stack_up.sh` (brings the stack up via `docker compose up -d`; polls aviso-server `/health`, NATS `/healthz`, and auth-o-tron's readiness endpoint with a 60-second bounded timeout and 250 ms poll interval). Add `tests/e2e/python/conftest.py` (session-scoped fixture calling `stack_up.sh` once per session per D-S2). No tests yet; just the scaffolding. Verify the helper from a manual run before the next commit lands.

3. **Commit 3: Python e2e suite**. All 10 Python test files; runs against the stack from Commit 2. Tests use the PR #21 API surfaces (`triggers=` kwarg + `with` / `async with` form). Identifier-space isolation per D-S7. Update `CONTRIBUTING.md` with the local-run paragraph for the Python suite.

4. **Commit 4: Rust e2e suite + final docs**. New `tests/e2e/rust/Cargo.toml` workspace member added to root `Cargo.toml`'s `[workspace] members`. Six Rust test files; every `#[test]` annotated `#[ignore = "requires e2e compose stack"]` per D-S6. Workspace gates (`cargo test --workspace --all-targets`, `cargo build --workspace --all-targets`, `cargo clippy --workspace --all-targets`) compile but skip running these tests; developers (and the future CI per `plans/e2e-ci-self-hosted.md`) opt in via `--include-ignored`. Update `CONTRIBUTING.md` with the local-run paragraph for the Rust suite. `python/examples/README.md` and `docs/src/python/quickstart.md` are EXPLICITLY OUT OF SCOPE per D-S9; they land in a separate follow-up PR after the e2e PR merges.

## What stays the same

- All current hermetic tests stay (Rust + Python).
- The doc-examples harness stays (the maintainer's local fact-check tool); only its server target becomes configurable.
- The public Python API surface from PR #21 is unchanged.
- The Rust core, CLI, and bindings are not touched by this work; only configuration, harness, and tests are added.

## What changes for users

Nothing user-facing in this branch. The CI signal does NOT change yet; the e2e suite is locally runnable but not gated. The wider CI integration is captured in `plans/e2e-ci-self-hosted.md` and lands when self-hosted runners are available on the repo.

The downstream win for end users (local examples via `docker compose up`) lands in a separate follow-up PR per D-S9; the present branch only ships the stack that the follow-up depends on.

## Status snapshot

- Branch: `feat/e2e-integration-suite`. Created off `main` at `5f16885`; PR #21 has since merged at `a0096d3`. The branch is rebased onto `a0096d3` before Commit 1 lands.
- Plan: this file. Three rounds of oracle plan review: two PROCEED rounds against `main` at `5f16885`; one AMEND-then-PROCEED round against `main` at `a0096d3` (post PR #21) that resolved workspace-member gating, auth flow specifics, history-gap mechanism, test isolation rule, in-memory variant drop, readiness polling, PR #21 API surfaces in tests, examples-update follow-up boundary, xdist drop, and the scope split between this branch (local-only) and `plans/e2e-ci-self-hosted.md` (CI on self-hosted runners).
- Not yet implemented.
- Image references: TBD (user to supply during implementation phase).
- Awaiting: user go-ahead for execution.
- Related plan: `plans/e2e-ci-self-hosted.md` (future work; wires this suite into CI on self-hosted runners with sccache + persistent docker image cache).
