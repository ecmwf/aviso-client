# End-to-end integration test suite

Replace the hermetic-only test posture with a real-services integration suite that exercises the supervisor, state store, auth flow, trigger dispatcher, and CLI subcommands against actual `aviso-server`, `auth-o-tron`, and NATS containers. Hermetic tests stay for fast feedback; the new suite catches the protocol and behaviour bugs the hermetic ones cannot.

## Motivation

The merged client suite (Rust core + CLI + Python bindings) is exercised today by:

- 400+ Rust unit tests using `wiremock` for HTTP and synthetic SSE streams.
- 110 Python tests that verify binding shape, value types, stub completeness, property tests on filters.
- A local doc-examples harness that runs every runnable Python code block against `aviso-server.ecmwf.int` (the ECMWF dev server).

Three behaviour classes are not covered by any of these:

1. **Real-wire reconnect on routine `max_duration_reached`**: wiremock cannot fake the `connection-closing` event the real server emits every `connection_max_duration_sec`, and the dev server's 1-hour rotation is too slow for tests.
2. **Authentication and authorization end-to-end**: aviso-server's direct-mode flow (forward Basic to auth-o-tron, validate, issue JWT, enforce roles per stream) cannot be faked by wiremock. Both branches of the auth surface need real services: 401 (wrong password, surfaces as `HttpError(status=401)`) needs auth-o-tron's plain provider to reject credentials, and 403 (valid credentials but role missing from `write_roles`, surfaces as `HttpError(status=403)`) needs aviso-server's schema-level role enforcement. The plan ships one Python scenario (`test_auth_errors.py`) covering both branches.
3. **CLI subcommands against a real server**: every CLI subcommand (`notify`, `listen`, `replay`, `schema list/get`, `admin wipe-*`, `config dump`) has only the wiremock surface from earlier work.

Auth refresh on real 401 was originally a fourth motivation. The plain auth-o-tron provider has no hot-reconfiguration mechanism (no SIGHUP, no file-watch, no admin API for credentials), so a deterministic mid-test 401 cannot be produced against real auth-o-tron without restarting the container with a swapped config (fragile and slow). Auth refresh is therefore covered by two hermetic tests instead: the existing supervisor wiremock test (`crates/aviso/tests/watch_supervisor_resilience.rs::auth_refresh_on_401_uses_refreshed_credential`) plus a new hermetic test for `ConfigFile::refresh()` (Commit 1).

The infrastructure to fix the above exists. `tests/e2e/docker-compose.yml` ships an `aviso-server` pinned by digest, with `connection_max_duration_sec` and `sse_heartbeat_interval_sec` knobs (`tests/e2e/aviso-server.config.yaml`) tuned short enough to surface reconnect bugs fast. The README explicitly notes: "The full e2e suite runs once real tests exist." This plan delivers that suite; the shipped values for those two knobs are documented in the compose stack additions section below.

## Reference: production configuration

The configs we ship for the test stack mirror ECMWF's production deployment so the e2e suite exercises real-world settings and the configs double as canonical examples for users who want to run their own deployment. Source repositories:

- [`ecmwf/aviso-chart`](https://github.com/ecmwf/aviso-chart) (Helm chart for aviso-server, with `examples/` profiles including [`values-auth-enabled.yaml`](https://github.com/ecmwf/aviso-chart/blob/main/examples/values-auth-enabled.yaml) and [`values-jetstream-internal-nats.yaml`](https://github.com/ecmwf/aviso-chart/blob/main/examples/values-jetstream-internal-nats.yaml)).
- [`ecmwf/aviso-config`](https://github.com/ecmwf/aviso-config) (production values overlay; [`location/bologna.yaml`](https://github.com/ecmwf/aviso-config/blob/main/location/bologna.yaml) is the live overlay we mirror).

The relevant production decisions we adopt verbatim:

- **Three accounts via auth-o-tron's `plain` provider**: `admin-user`, `reader-user`, `producer-user`, each mapped to one role (`admin`, `reader`, `producer`) by a `plain-role-augmenter`.
- **Direct-mode aviso-server auth**: aviso-server forwards Basic to auth-o-tron's `/authenticate`, receives a JWT in the response header, validates it locally on subsequent requests via a shared JWT secret.
- **`test_polygon` schema has `auth.required: true` with `write_roles: ["producer"]`**: any authenticated user can read; only `producer-user` can publish.
- **Shared JWT secret** between aviso-server (`AVISOSERVER_AUTH__JWT_SECRET` env) and auth-o-tron (`AOT_JWT__SECRET` env); in the test stack this is a hardcoded value in `docker-compose.yml`, not a Kubernetes secret.
- **JetStream backend**: aviso-server creates the stream with bounded `max_messages` (so `test_history_gap` can deterministically force pruning) and `discard_policy: old`.

## What ships

### Compose stack additions

The existing `tests/e2e/docker-compose.yml` gains two services and the `aviso-server` config flips from in-memory + no-auth to NATS-backed + auth-o-tron-authenticated:

- **`nats`**: JetStream-enabled NATS server (image `nats:2.12.4-alpine@sha256:<digest>` from Docker Hub, matching the version the upstream NATS Helm chart bundles in `aviso-chart/charts/nats-2.12.4.tgz`). `nats.conf` enables the monitoring server on port `8222` for readiness polling and JetStream with bounded file-store. Exposed on `127.0.0.1:${NATS_HOST_PORT:-4222}` (client port) and `127.0.0.1:${NATS_MONITORING_HOST_PORT:-8222}` (monitoring; the `/healthz` endpoint the readiness helper polls). The aviso-server side configures the stream with a small `max_messages` so `test_history_gap.py` can deterministically force pruning by publishing past the limit.
- **`auth-o-tron`**: ECMWF's auth service (image `eccr.ecmwf.int/auth-o-tron/auth-o-tron:0.3.3@sha256:<digest>` from ECMWF's registry, matching the chart default in `auth-o-tron-chart-0.3.1/values.yaml`). Config mirrors `aviso-config/location/bologna.yaml` verbatim modulo `iss` and `aud` strings (set to `authotron-test`). The plain provider declares three users (`admin-user` / `admin-pass`, `reader-user` / `reader-pass`, `producer-user` / `producer-pass`); the plain-role-augmenter maps each to one role (`admin`, `reader`, `producer`). Exposed on `127.0.0.1:${AUTH_O_TRON_HOST_PORT:-8080}`. The `/health` endpoint is the readiness check.

`aviso-server.config.yaml` flips to:

- `notification_backend.kind: jetstream` with `nats_url: "nats://nats:4222"`, `max_messages: 100`, `discard_policy: old`, `storage_type: file`, `retention_time: "1h"`, `enable_auto_reconnect: true`.
- `auth.enabled: true`, `auth.mode: direct`, `auth.auth_o_tron_url: "http://auth-o-tron:8080"`, `auth.admin_roles: { localrealm: ["admin"] }`, `auth.timeout_ms: 5000` (verbatim from `aviso-config/environment/test.yaml`).
- `watch_endpoint.connection_max_duration_sec: 15` (down from 60). The shorter cycle keeps reconnect tests deterministic without dominating wall time: 15 seconds is long enough that a publish-and-listen pair completes inside one connection (no false cuts in non-reconnect tests) and short enough that `test_reconnect.py` pays ~20 s wall-time instead of ~70.
- `watch_endpoint.sse_heartbeat_interval_sec: 5` (already in the existing config).
- `notification_schema` block holds `test_event` and `test_polygon`, both with `auth.required: true` and `write_roles: { localrealm: ["producer"] }` matching the bologna overlay's `test_polygon`. The schema subtree now carries the production-style `auth:` block; the quickstart prose explaining that block is deferred to the follow-up examples PR per D-S9. The PR #21 quickstart snippet itself sources from this file but is not modified in this branch.
- `notification_schema_strict: true` matches the production setting.

aviso-server's JWT secret is injected via `AVISOSERVER_AUTH__JWT_SECRET` env in docker-compose, paired with auth-o-tron's `AOT_JWT__SECRET` env at the same value (the shared-secret pattern from `aviso-config`). The value is a hardcoded `aviso-e2e-test-secret-do-not-use-in-production` (or similar; it never leaves the local stack).

The compose file gains `nats.conf` and `auth-o-tron.config.yaml` mounted into their respective containers (`/etc/nats/nats.conf` and `/app/config.yaml` per the chart templates).

### Test harness layout

```text
tests/e2e/
├── docker-compose.yml              (extended: + nats + auth-o-tron + env vars)
├── aviso-server.config.yaml        (NATS backend, auth enabled, bounded stream)
├── auth-o-tron.config.yaml         (NEW: 3 users, plain provider + role augmenter)
├── nats.conf                       (NEW: JetStream + monitoring on 8222)
├── README.md                       (extended)
├── shared/
│   └── stack.sh                    (NEW: up | down | restart | logs | status dispatcher)
├── python/                         (NEW)
│   ├── conftest.py                 (session-scoped stack fixture; shared stack)
│   ├── test_publish_listen.py
│   ├── test_reconnect.py
│   ├── test_auth_errors.py
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
    │   └── lib.rs                  (shared base URL + credential constants + producer/reader client constructors)
    └── tests/
        ├── library_publish_listen.rs
        ├── library_reconnect.rs
        ├── cli_publish.rs
        ├── cli_listen_yaml.rs
        └── cli_replay_history.rs
```

The Python suite is the primary user-facing validator. The Rust suite covers library API ergonomics and CLI behaviour that the Python suite cannot reach.

### Test scenarios

#### Python suite (`tests/e2e/python/`)

Ten scenarios targeting the user-facing API. Tests use the PR #21 API surfaces: `client.listen(..., triggers=[...])` with the `triggers=` kwarg, and `with client.listen(...) as iterator:` / `async with client.listen(...) as iterator:` for clean shutdown. Identifier-space isolation per D-S7 (unique polygons or `(date, time)` pairs per test). All tests authenticate as `producer-user:producer-pass` unless the scenario explicitly tests another role.

| File | What it asserts |
|---|---|
| `test_publish_listen.py` | Publish N notifications, listen for them via `with client.listen(...) as iterator:`, assert all N received in order with matching identifier and payload. Uses `producer-user`. |
| `test_reconnect.py` | Open a listener, publish two notifications, wait past `connection_max_duration_sec` (15 s), publish two more, assert all four arrive across the routine `max_duration_reached` cut with at-most-one duplicate per cut (D2). Uses `producer-user`. A separate scenario for `docker compose restart aviso-server` mid-stream is captured as future work in the related CI plan; the routine-close path is the more frequent reconnect cause and the one the supervisor classifier optimises for. |
| `test_auth_errors.py` | Two test functions in one file. (a) `test_invalid_credentials_get_401`: authenticate as `reader-user` with wrong password; call `client.notify(...)`; assert `aviso.HttpError` with `status == 401`. (b) `test_role_mismatch_gets_403`: authenticate as `reader-user` (role `reader`) with the right password; try to `client.notify(...)` on `test_polygon` (`write_roles: ["producer"]`); assert `aviso.HttpError` with `status == 403`. The client does not have a `PermissionDenied` variant: 401 and 403 both surface as `HttpError` with the status code attached (see `python/aviso/__init__.pyi::HttpError`). Replaces the original `test_auth_refresh.py` per the motivation rewrite above. |
| `test_resume_across_restart.py` | Listen with `JsonFileStore`, consume N notifications, close the first `AvisoClient` (in-process simulation of a restart), open a second `AvisoClient` against the same state file, publish N+1, assert N+1 arrives. The supervisor's commit-on-next-send semantics mean item N may replay once (at-least-once contract); item 1 must never replay (already committed). True process-kill-and-restart is not exercised at the pytest level because the fixture's process scope would leak; the in-process variant covers the `JsonFileStore` round-trip contract that is the actual library surface under test. Uses `producer-user`. |
| `test_history_gap.py` | **SKIPPED** at landing. The intended scenario publishes past the configured `max_messages: 100`, requests replay from a known pruned sequence, and asserts `HistoryGapError(reason="replay_limit_reached")` with `.max_allowed` populated. Skipped because aviso-server 0.6.2 silently starts replay from the oldest available sequence instead of emitting `notification_replay_limit_reached` when `from_id` is below the retained range; the server-side trigger conditions need a focused dig before this test can be written reliably. The Rust supervisor's hermetic test at `crates/aviso/src/watch/supervisor/tests/drain_mapping.rs::drain_frames_mapping_terminates_on_replay_limit_reached_with_history_gap` already verifies the client side handles the signal. |
| `test_triggers_real_dispatch.py` | `client.listen(..., triggers=[Trigger.echo(), Trigger.log(path), Trigger.webhook(url)])` with the local webhook pointed at an in-process `pytest-httpserver`. Assert stdout, file contents, and webhook receipts all match expected. Uses `producer-user`. |
| `test_multiplex_async.py` | Two `async with client.listen(...) as iterator:` calls under `asyncio.gather`; assert per-stream ordering preserved, cross-stream interleaving is non-deterministic. Uses `producer-user`. |
| `test_flush_cursor_on_exit.py` | `flush_cursor_on_exit=True` + iterator `with` block; clean shutdown, then restart; assert no replay of the last notification. Uses `producer-user`. |
| `test_schema_discovery.py` | `client.schema()` + `client.schema_for(...)` against the test stack; assert returned shape matches the configured schemas. Uses `reader-user` (schema endpoint requires auth but no specific role). |
| `test_concurrent_publishers.py` | Open a listener, then fire five `asyncio.gather`'d publishes, then drain the live stream; assert all five `request_id`s are unique and all five payload `seq` values arrive on the listener. The listen-before-publish order avoids the supervisor's subscribe race; the test asserts the concurrent-publish path produces distinct request_ids and that nothing is lost on the wire. Uses `producer-user`. |

#### Rust suite (`tests/e2e/rust/`)

Five scenarios targeting Rust-specific surfaces. Every `#[test]` function carries `#[ignore = "requires e2e compose stack"]` so workspace-wide `cargo test` invocations compile the crate but skip running these tests; developers (and the future CI per `plans/e2e-ci-self-hosted.md`) opt in via `cargo test -p aviso-e2e -- --include-ignored`.

| File | What it asserts |
|---|---|
| `library_publish_listen.rs` | `AvisoClient::builder()` + `client.notify(...).await?` + `client.watch(req)?` against the stack. Validates the Rust API ergonomics, not just the binding. Uses `producer-user` via `BasicAuth`. |
| `library_reconnect.rs` | Same reconnect scenario as the Python test, from the Rust library caller's POV. Catches Rust-API-specific bugs (lifetimes, error propagation, supervisor lifetime) that the binding might hide. |
| `cli_publish.rs` | `aviso notify` subcommand with `--base-url`/`--username`/`--password` flags and a quoted polygon parameter; asserts the command exits 0. Validates argument parsing + the parameter-string format end-to-end. |
| `cli_listen_yaml.rs` | `aviso listen <yaml>` with a listener YAML configuring an echo trigger; spawns the listener as a child process, publishes three notifications via `aviso notify`, asserts captured stdout contains the published NDJSON. Echo is the cheapest representative trigger; expanding to every kind would require log-file / webhook / command targets and per-trigger output assertions, none of which add coverage the per-trigger hermetic tests do not already give. |
| `cli_replay_history.rs` | `aviso replay --from <future-seq>` (u64::MAX - 1) exits 0 with an empty replay window. Picking a future sequence keeps the test insensitive to the shared JetStream stream's accumulated state across other tests in the session, while still exercising the request-build, auth, and end-of-stream paths. |

The original `library_auth_refresh.rs` is dropped (auth refresh is covered hermetically per the motivation rewrite; the Rust-side surfaces tested are already exercised by `library_publish_listen.rs` + the new hermetic `ConfigFile::refresh()` test in Commit 1).

### CI integration (deferred)

CI integration is OUT OF SCOPE for this branch and is deferred to `plans/e2e-ci-self-hosted.md`. The reason: a GitHub-hosted `ubuntu-latest` runner would add ~3-5 min warm / ~6-9 min cold per PR (docker image pulls, cold Rust builds, the reconnect-test floor). The right answer is a self-hosted runner with persistent docker image cache plus `sccache` against the user's existing S3 bucket, getting the warm cycle to ~1-2 min. That work is blocked on assigning self-hosted runners to the repo, which the user is taking offline with a colleague.

What stays in CI in this branch:

- The existing `e2e-config` job (`docker compose -f tests/e2e/docker-compose.yml config --quiet`) keeps validating the docker-compose file's syntactic correctness. It runs on every PR and must stay green after Commit 2's compose-stack changes land.
- Nothing else CI-related changes. The e2e suite (Python + Rust) is locally runnable but not gated.

What this branch DOES ship for local use:

- The compose stack, the readiness-polling helper (`tests/e2e/shared/stack.sh up`), and a documented per-language flow. Python: `bash tests/e2e/shared/stack.sh up && uv run pytest tests/e2e/python/`. Rust: `bash tests/e2e/shared/stack.sh up && cargo build -p aviso-cli && cargo test -p aviso-e2e -- --include-ignored --test-threads=1` (the build step is required because the CLI tests use `assert_cmd::Command::cargo_bin("aviso")` which expects `target/debug/aviso`; `--test-threads=1` keeps publishers and listeners from racing in the shared JetStream stream).
- The Rust e2e tests are `#[ignore]`-gated per D-S6 so the existing `cargo test --workspace --all-targets` CI gate keeps passing without the stack up. Developers explicitly opt in via `cargo build -p aviso-cli && cargo test -p aviso-e2e -- --include-ignored --test-threads=1` (the build step seeds `target/debug/aviso` for the CLI tests; `--test-threads=1` serializes the shared-stack publishers and listeners).
- The Python e2e tests live under `tests/e2e/python/` and are NOT picked up by the hermetic `pyproject.toml`'s `[tool.pytest.ini_options] testpaths = ["python/tests"]`. Developers run them explicitly via `uv run pytest tests/e2e/python/`.

### What this changes about the existing test surface

- **Hermetic Python tests stay** (`python/tests/`). Fast feedback loop (~1.5 s) remains. These exercise the API shape; the e2e suite exercises the behaviour. `pyproject.toml`'s `[tool.pytest.ini_options] testpaths = ["python/tests"]` stays narrow so the e2e directory is never picked up by the hermetic run.
- **Hermetic Rust tests stay** (`crates/aviso/tests/`, `crates/aviso-cli/tests/`). The supervisor's classifier tests, the SSE parser tests, the wiremock-driven supervisor stress tests all stay. The new Rust e2e suite is additive, not a replacement.
- **`ConfigFile` gains a re-read-on-`refresh()` capability with hermetic test** in Commit 1. Today `ConfigFile` reads its file once at `from_path` and the trait's default `refresh()` is a no-op; this is a real bug for any user who relies on the documented refresh-then-retry-once contract with a file-backed credential source. Fixing it is a prerequisite for the doc claims in `plans/decisions.md` D8 to hold. The fix is small (store the path; wrap `inner` in `tokio::sync::RwLock<ConfigSource>`; override `refresh()` to re-read; preserve the existing `from_yaml_str` constructor for tests that pass YAML directly, with `refresh()` a no-op for that constructor).
- **The doc-examples harness** (the maintainer's `/tmp/aviso-py-fact-check/run_doc_examples.py` script that is not part of CI) becomes server-agnostic via `AVISO_BASE_URL`; after the stack lands the harness can point at `http://localhost:8000` instead of `aviso-server.ecmwf.int`. The harness now also needs to set `AVISO_USERNAME` and `AVISO_PASSWORD` because the test schema requires auth.
- **The PR #21 schema snippet in `docs/src/python/quickstart.md`** continues to source from `tests/e2e/aviso-server.config.yaml`. The file's path does not change; the `notification_schema:` subtree the snippet sources from now also carries the `auth:` block (mirroring production). The quickstart prose around the snippet is updated in the follow-up examples PR (per D-S9) to explain the auth block; this branch does not touch the quickstart prose.

### What this changes for the python/examples/ tree (D-S9)

This is one of the highest-leverage downstream wins of the e2e work and the entire reason we are doing it from a user-experience angle.

Today, the 15 example scripts under `python/examples/` assume the reader's server has the `test_polygon` event type configured. We document a fallback ("substitute your own event type if not") and ship a copy-paste schema snippet for operators, but a new user without an operator account on any aviso-server still has friction: they have to find a server, get credentials, hope it has `test_polygon`, and only then can they try the examples.

Once the e2e stack ships, that friction collapses to three commands run from the repo root:

```bash
bash tests/e2e/shared/stack.sh up
export AVISO_BASE_URL=http://localhost:8000 \
       AVISO_USERNAME=producer-user \
       AVISO_PASSWORD=producer-pass
python python/examples/basics/01_publish.py
```

The `test_polygon` schema is already in `tests/e2e/aviso-server.config.yaml`, so every example runs out of the box against the local stack with no operator involvement.

**The examples README update and the quickstart rewrite are explicitly OUT OF SCOPE for this branch.** They land in a separate follow-up PR after the e2e PR merges. This boundary keeps the e2e PR focused on the stack + the test suite + the library fix, and lets the user verify the stack works end-to-end before committing to the examples-rewrite shape. The follow-up will:

- Add a "Run the examples against the local stack" subsection to `python/examples/README.md` showing the commands above.
- Add a one-line cross-reference from `docs/src/python/quickstart.md`'s "What is on your server" section so the schema-snippet path frames itself as "two options: spin up the local stack, or paste this into your own server's config (and configure auth)".
- Optionally promote `triggers/05_webhook.py` to runnable now that the harness has a local HTTP receiver pattern in `advanced/03_webhook_with_local_server.py`; reconsider whether the split-into-two-files pedagogy still makes sense once both are runnable.

The follow-up's scope is small (a docs edit and possibly the webhook-pair reconsideration); tracked here so it does not get lost.

## Decisions

### D-S1. Same compose stack serves both language suites

One compose file, one stack-up call, both languages run against it sequentially in a local or future-CI run. Two stacks would be wasteful and would introduce cross-stack drift.

### D-S2. Single session-scoped stack per pytest session

`tests/e2e/python/conftest.py` brings the stack up once via `bash tests/e2e/shared/stack.sh up` at session scope, holds it for every test in the session, and LEAVES IT RUNNING after the last test so successive `uv run pytest` invocations skip the docker startup cost. Setting `AVISO_E2E_TEARDOWN=1` opts the fixture in to running `tests/e2e/shared/stack.sh down` at session end. No pytest-xdist sharding in the initial implementation: the ten Python scenarios run sequentially with the reconnect-test floor at ~20 s, comfortably within a single local run, and adding xdist would require port-sharding logic plus the `pytest-xdist` dev dependency for no concrete payoff. The existing `tests/e2e/README.md` parallel-shard pattern (`AVISO_SERVER_HOST_PORT=8101 -p shard-1`) survives as documentation for human operators who want to run the suite locally alongside other instances; the test fixture itself does not exercise it. If a future test count growth makes the suite slow enough to warrant parallelism, xdist + per-worker port sharding lands as a small follow-up.

### D-S3. JetStream-backed NATS

In-memory NATS would skip persistence semantics that real users care about (resume across NATS restart, retention pruning). JetStream is the only option that exercises the full backend. The chart-bundled `nats:2.12.4-alpine` image is the one we use, matching the version `aviso-chart/charts/nats-2.12.4.tgz` ships.

### D-S4. Real auth-o-tron mirroring production; auth refresh stays hermetic

The e2e suite exercises authentication and authorization end-to-end against real auth-o-tron, configured to mirror the production overlay (`aviso-config/location/bologna.yaml`). The plain provider declares three users with distinct roles (`admin-user`/admin, `reader-user`/reader, `producer-user`/producer); aviso-server runs in `direct` auth mode (D8) and enforces per-stream `read_roles` / `write_roles` plus admin-endpoint role gating via `admin_roles`. The shared JWT secret pattern (`AVISOSERVER_AUTH__JWT_SECRET` paired with `AOT_JWT__SECRET`) is preserved.

Auth refresh on real 401 is NOT covered by this suite, because the plain auth-o-tron provider has no hot-reconfiguration mechanism (no SIGHUP, no file-watch, no admin API for credentials, no per-token TTL). The only ways to mid-test invalidate a credential against real auth-o-tron are container restart with a swapped config (fragile, slow, races aviso-server's auth-validation cache) or adding a MongoDB-backed token store (significant scope expansion to a fourth service for one test).

Instead, auth refresh is covered by two hermetic tests:

- `crates/aviso/tests/watch_supervisor_resilience.rs::auth_refresh_on_401_uses_refreshed_credential` (existing): wiremock-driven, asserts the supervisor's refresh-then-retry-once contract delivers the refreshed credential on the second attempt against a 401 → 200 sequence.
- `crates/aviso/src/auth/config_file.rs::tests::*` (new, Commit 1): a small set of unit tests asserting `ConfigFile` re-reads the file on `refresh()` and atomically swaps `inner`, so a file rewrite between requests is reflected on the next `authorization_header()` call. Specifically: (i) `refresh_rereads_file_after_rewrite` swaps a Bearer token for a different Bearer; (ii) `refresh_swaps_section_kind` swaps from a Bearer section to a Basic section; (iii) `refresh_parse_failure_leaves_previous_credential` verifies that a write-then-bad-yaml round-trip leaves the old `inner` active and surfaces `ClientError::Auth` from the refresh (the invariant is "parse the new YAML into a fresh `ConfigSource` BEFORE acquiring the write lock, then perform a single swap"); (iv) `refresh_is_noop_for_from_yaml_str_constructor` verifies the in-memory constructor path stays a no-op.

This split is the right trade: hermetic tests get the right granularity (file-rewrite + 401 + retry-once), and the e2e suite focuses on the behaviours where real services matter (reconnect, JetStream retention, role enforcement, CLI subcommands).

### D-S5. Image versions pinned by digest

The existing `aviso-server` is pinned `0.6.2@sha256:...`. The new `nats` and `auth-o-tron` services follow the same convention: tag for human readability, digest for reproducibility. Initial tags: `eccr.ecmwf.int/auth-o-tron/auth-o-tron:0.3.3` (from `auth-o-tron-chart-0.3.1/values.yaml`) and `nats:2.12.4-alpine` (from `aviso-chart/charts/nats-2.12.4.tgz`). Digests resolved at Commit 2 implementation time via `docker buildx imagetools inspect`. README documents the bump procedure.

### D-S6. Rust e2e suite is a workspace member with `#[ignore]`-gated tests

`tests/e2e/rust/Cargo.toml` declares an `aviso-e2e` package. The workspace `Cargo.toml` adds it as a member so `cargo build --workspace --all-targets`, `cargo clippy --workspace --all-targets`, and `cargo test --workspace --all-targets` (the gates the existing CI Rust job runs on every PR) all compile the e2e crate's test code and catch type errors. Every `#[test]` function in the crate carries `#[ignore = "requires e2e compose stack"]`, so the normal `cargo test --workspace` invocation skips actually running them. The dedicated e2e job opts in via `cargo test --locked -p aviso-e2e -- --include-ignored`.

The e2e crate's `Cargo.toml` declares:

- `publish = false` (this crate never ships to crates.io; it is internal test infrastructure).
- `[lints] workspace = true` (the same clippy and rustc lint level applied to every workspace crate applies here too; no quiet exceptions).
- `[lib] doctest = false` on the shared helper in `src/lib.rs`. The helper is internal scaffolding (base URL accessor, credential constants, producer/reader client constructors) with no public API users would write docs against, so the `cargo test --workspace --doc` gate has nothing to run here.

These three settings together prevent the workspace-member relationship from leaking unintended gates onto the e2e crate.

The alternative (a separate top-level Cargo project outside the workspace) was considered and rejected because workspace membership keeps the dev-dep deduplication and lets the e2e crate import the core via a path dependency (`aviso = { path = "../../../crates/aviso", version = "=0.1.0" }`, matching the existing aviso-cli and aviso-py pattern); the per-test `#[ignore]` annotation costs nothing in maintenance and gives a clear "why this is skipped" signal in `cargo test` output.

`cargo deny check` already covers any new dev-dependencies the e2e crate brings; no separate deny.toml entry is required. `Cargo.lock` updates land in the same commit (Commit 4) that adds the e2e crate as a workspace member.

### D-S7. Tests isolate via disjoint identifier values, not unique event types

The stack ships two event-type schemas in `tests/e2e/aviso-server.config.yaml` (`test_event` and `test_polygon`); inventing a per-test event type would require either runtime schema registration (not supported by aviso-server) or extra `notification_schema` entries in the test config (maintenance overhead for no payoff). Instead, each test reserves a disjoint slice of the identifier space and filters its watch to that slice:

- For `test_polygon`, each test uses a unique polygon string (the polygon serves as a discriminator that the filter can pin; the schema declares polygon `required: true` so it appears in every notify and every watch filter).
- For `test_event`, each test uses a unique `(date, time)` pair.

The admin wipe endpoint is reserved for tests that intentionally mutate global server state as the test under test (e.g., a test for `aviso admin wipe-stream` itself); routine isolation does not need it. This keeps the suite order-independent and parallelisable later without per-test cleanup machinery.

The stack-up cost is paid once per session: `bash tests/e2e/shared/stack.sh up` brings up all three services and polls for readiness, then the session-scoped fixture in `tests/e2e/python/conftest.py` holds the stack for every test in the session.

### D-S8. Failure logs are captured for local debugging

On any Python e2e test failure during a local run, `tests/e2e/python/conftest.py`'s `pytest_sessionfinish` hook captures `docker compose -f tests/e2e/docker-compose.yml logs > tests/e2e/last-failure.log` (gitignored). The Rust suite does not currently auto-capture logs on failure; on a Rust-side failure, the operator inspects `docker compose -f tests/e2e/docker-compose.yml logs` manually (the suite leaves the stack running by default). The future CI workflow (`plans/e2e-ci-self-hosted.md`) extends the capture pattern to cover both suites and uploads `last-failure.log` as an artifact.

### D-S9. The same compose stack is the recommended local environment for running the `python/examples/` tree

The e2e suite is the primary consumer of the stack, but the stack is reusable for any local validation. Specifically: the `python/examples/` scripts assume a server with `test_polygon` configured and (after this branch) require credentials with the `producer` role to publish; the e2e stack ships exactly that. A new user evaluating the Python API will be told to run the same `bash tests/e2e/shared/stack.sh up` that the test suite uses, set `AVISO_BASE_URL=http://localhost:8000` + `AVISO_USERNAME=producer-user` + `AVISO_PASSWORD=producer-pass`, and run any example unchanged. This collapses the "find a server, get credentials, hope it has test_polygon" friction to three commands.

**The follow-up examples PR documents this; the e2e work in THIS branch does not touch `python/examples/README.md` or `docs/src/python/quickstart.md`.** The e2e branch ships the stack + the test suite + the library fix. The examples-and-quickstart rewrite is a separate, smaller PR after the e2e PR merges. Keeping the two PRs separate lets the user verify the stack works end-to-end before committing to the examples-rewrite shape, and keeps the e2e PR focused on one PR-level concern per AGENTS.md.

## Open questions

1. **Exact image digests for `auth-o-tron:0.3.3` and `nats:2.12.4-alpine`**. Tags are pinned per D-S5; digests are resolved at Commit 2 implementation time via `docker buildx imagetools inspect <image>:<tag>` (the same procedure documented in `tests/e2e/README.md` for the existing aviso-server pin).

## Roll-out

Five focused commits on `feat/e2e-integration-suite`. The branch is rebased onto `main` (`a0096d3`) as of the third oracle round; see Status snapshot.

1. **Commit 1: `feat(aviso): ConfigFile re-reads credentials on refresh()`**. Library-level fix: store the path on `ConfigFile`, wrap `inner` in `tokio::sync::RwLock<ConfigSource>`, override `refresh()` to (a) read the file, (b) parse YAML into a fresh `ConfigSource` (so parse failure cannot leave a torn state), and (c) acquire the write lock and swap in one operation. Preserve the existing `from_yaml_str` constructor for unit tests that pass YAML directly; its `refresh()` is a no-op (the stored path is `None`). Add hermetic tests: `refresh_rereads_file_after_rewrite`, `refresh_swaps_section_kind` (bearer → basic), `refresh_parse_failure_leaves_previous_credential` (parse-before-swap invariant), `refresh_is_noop_for_from_yaml_str_constructor`. ~80 LOC + ~4 new tests on top of the existing 13. Replaces the deferred-to-implementation framing of the original plan with a concrete library fix.

2. **Commit 2: compose stack + configs + local-run README**. Extend `docker-compose.yml` with `auth-o-tron` and `nats` services pinned by `<tag>@<digest>`; add `auth-o-tron.config.yaml` mirroring `aviso-config/location/bologna.yaml` modulo the test-local `iss`/`aud` strings; add `nats.conf` enabling monitoring on 8222 + JetStream with bounded file-store; flip `aviso-server.config.yaml` to `notification_backend.kind: jetstream`, `auth.enabled: true`, `connection_max_duration_sec: 15`, schema `auth.required: true` with `write_roles: ["producer"]`; set the shared JWT secret via env vars in docker-compose. Add `tests/e2e/shared/stack.sh up` (polls aviso-server `/health`, NATS `/healthz` on 8222, auth-o-tron's `/health` with a 60 s bounded timeout and 250 ms poll interval). Update `tests/e2e/README.md` to document the new shape, the bump procedure for the two new pinned images, the local-run workflow (`bash tests/e2e/shared/stack.sh up && export AVISO_BASE_URL=... && export AVISO_USERNAME=producer-user && export AVISO_PASSWORD=producer-pass`), and the three account credentials. The existing `e2e-config` CI gate (`docker compose config --quiet`) must pass against the new shape.

3. **Commit 3: Python conftest + e2e suite**. Add `tests/e2e/python/conftest.py` (session-scoped fixture calling `stack.sh up` once per session per D-S2; failure-teardown captures docker logs per D-S8). Add the 10 Python test files; tests use the PR #21 API surfaces (`triggers=` kwarg + `with` / `async with` form). Identifier-space isolation per D-S7. Update `CONTRIBUTING.md` with the local-run paragraph for the Python suite.

4. **Commit 4: Rust e2e suite + workspace wiring**. New `tests/e2e/rust/Cargo.toml` workspace member added to root `Cargo.toml`'s `[workspace] members`. Shared helper in `tests/e2e/rust/src/lib.rs` (base URL accessor, credential constants, producer/reader client constructors, `isolated_aviso_command` helper for CLI subprocess isolation; stack startup is handled by `tests/e2e/shared/stack.sh up`). Five Rust test files; every `#[test]` annotated `#[ignore = "requires e2e compose stack"]` per D-S6. Workspace gates compile but skip running these tests; developers (and the future CI per `plans/e2e-ci-self-hosted.md`) opt in via `cargo build -p aviso-cli && cargo test -p aviso-e2e -- --include-ignored --test-threads=1`. Update `CONTRIBUTING.md` with the local-run paragraph for the Rust suite.

5. **Commit 5: final docs polish**. Update `README.md` with a sentence pointing at the e2e suite and the local-run docs. Cross-check `tests/e2e/README.md` is complete after commits 2-4 layered changes. `python/examples/README.md` and `docs/src/python/quickstart.md` are EXPLICITLY OUT OF SCOPE per D-S9; they land in a separate follow-up PR after the e2e PR merges.

## What stays the same

- All current hermetic tests stay (Rust + Python).
- The doc-examples harness stays (the maintainer's local fact-check tool); only its server target and credentials become configurable.
- The public Python and Rust API surfaces from PR #21 are unchanged. Commit 1's `ConfigFile::refresh()` fix is a behavioural change inside the existing `AuthProvider` contract (refresh used to be a no-op; now it actually refreshes), but the public API shape is the same.
- The Rust core, CLI, and bindings are not otherwise touched by this work; only configuration, harness, the `ConfigFile` library fix, and tests are added.

## What changes for users

No new public API shape ships. One existing behaviour improves: `ConfigFile::refresh()` now re-reads file-backed credentials per its docstring, where previously it silently inherited the trait's default no-op (a latent bug for any user with a file-backed credential source going through the supervisor's refresh-then-retry-once contract).

The CI signal does NOT change in this branch; the e2e suite is locally runnable but not gated. The wider CI integration is captured in `plans/e2e-ci-self-hosted.md` and lands when self-hosted runners are available on the repo.

The downstream win for end users (local examples via `docker compose up` + creds env vars) lands in a separate follow-up PR per D-S9; the present branch only ships the stack and library fix that the follow-up depends on.

## Status snapshot

- Branch: `feat/e2e-integration-suite`. Implementation landed in PR #22 against `main` at `a0096d3`.
- Final shape after implementation: e2e auth-refresh test was dropped (auth-o-tron's plain provider has no hot-reconfiguration mechanism that produces a deterministic mid-test 401 against real services; auth refresh is covered hermetically instead by the supervisor wiremock test plus a new `ConfigFile::refresh()` library fix that shipped as the first commit of the series). The stack mirrors the bologna overlay verbatim with three accounts (`admin-user` / `reader-user` / `producer-user`) mapped to three roles via `plain-role-augmenter`. The auth-errors scenario asserts `HttpError(status=401)` and `HttpError(status=403)` because the client does not expose a typed `PermissionDenied` variant.
- Test results at landing: 11 Python e2e scenarios pass, 1 skipped (`test_history_gap`, with documented investigation note); 5 Rust e2e scenarios pass.
- Image references: `eccr.ecmwf.int/auth-o-tron/auth-o-tron:0.3.3@sha256:380afb697e086fd8f6d64ab2cdcb0583d1603a2e29bccd8e950f90bbaf9bfe66` and `nats:2.12.4-alpine@sha256:31c6ed3b2da61645aaa3ad9217b5a52b34b6ebd555ecb71259cd7723c59ae1ea`.
- Related plan: `plans/e2e-ci-self-hosted.md` (future work; wires this suite into CI on self-hosted runners with sccache + persistent docker image cache).
