# End-to-end tests

E2E tests run against a real three-service stack: `aviso-server` + `auth-o-tron` + JetStream-backed NATS, all pulled from public registries and pinned by manifest-index digest for reproducibility:

```text
eccr.ecmwf.int/aviso/aviso_server:0.6.2@sha256:2c2607d1ba4d9b4bf55e52bf9b08793d8ca1959843a3efaa3d6f6922d5fdc60c
eccr.ecmwf.int/auth-o-tron/auth-o-tron:0.3.3@sha256:380afb697e086fd8f6d64ab2cdcb0583d1603a2e29bccd8e950f90bbaf9bfe66
nats:2.12.4-alpine@sha256:31c6ed3b2da61645aaa3ad9217b5a52b34b6ebd555ecb71259cd7723c59ae1ea
```

The tag (e.g. `0.6.2`) is a human-readable label; the digest is what Docker resolves and verifies. A re-published tag cannot silently change CI or local runs because the digest will no longer match.

The stack mirrors ECMWF's production deployment patterns (sourced from [`ecmwf/aviso-chart`](https://github.com/ecmwf/aviso-chart) and [`ecmwf/aviso-config`](https://github.com/ecmwf/aviso-config)):

- **aviso-server** runs with `auth.enabled: true`, `auth.mode: direct`, JetStream backend, and two schemas (`test_event`, `test_polygon`) both with `auth.required: true` and `write_roles: ["producer"]`.
- **auth-o-tron** runs the `plain` provider with three test accounts mapped to three roles via a `plain-role-augmenter`.
- **nats** runs JetStream with bounded `max_messages` (sized to support a future deterministic history-gap test once aviso-server's `notification_replay_limit_reached` trigger condition is pinned down; the current `test_history_gap.py` is skipped) and the monitoring server on port `8222`.
- aviso-server and auth-o-tron share a JWT secret via the `AVISOSERVER_AUTH__JWT_SECRET` and `AOT_JWT__SECRET` env vars (default value baked into `docker-compose.yml`; override via `AVISO_E2E_JWT_SECRET`).

## Test accounts

| Username | Password | Role | Use case |
|---|---|---|---|
| `admin-user` | `admin-pass` | `admin` | Admin endpoints (`/api/v1/admin/*`). |
| `reader-user` | `reader-pass` | `reader` | Listen / replay against streams. Cannot publish (will get 403). |
| `producer-user` | `producer-pass` | `producer` | Publish + listen + replay. The default account for the e2e suite. |

## Run the suite locally

The stack-up helper polls each service's readiness endpoint before returning, then pytest and cargo can run against the live stack:

```bash
# from the repo root

# one-time setup (or whenever crates/aviso-py changes)
uv sync --locked --group dev
uv run maturin develop --locked

# bring the stack up
bash tests/e2e/shared/stack.sh up

# python e2e suite
export AVISO_BASE_URL=http://localhost:8000 \
       AVISO_USERNAME=producer-user \
       AVISO_PASSWORD=producer-pass
uv run pytest tests/e2e/python/

# rust e2e suite (tests are #[ignore]-gated; --include-ignored opts in;
# --test-threads=1 keeps publishers and listeners from racing in the shared stream)
cargo build -p aviso-cli
cargo test --locked -p aviso-e2e -- --include-ignored --test-threads=1

# teardown
bash tests/e2e/shared/stack.sh down
```

`stack.sh` is the single lifecycle entry point. Other useful subcommands:

```bash
bash tests/e2e/shared/stack.sh status   # ps + readiness probes
bash tests/e2e/shared/stack.sh restart  # wipe JetStream and bring back up
bash tests/e2e/shared/stack.sh logs     # tail the last 100 lines of docker compose logs
```

The `uv sync` + `uv run maturin develop` setup builds the `aviso._native` extension into the local venv so the Python suite can `import aviso`; without it, `uv run pytest tests/e2e/python/` fails on first import. The `cargo build -p aviso-cli` step is required because the Rust e2e CLI tests use `assert_cmd::Command::cargo_bin("aviso")`, which expects the binary to exist at `target/debug/aviso`.

Per-pytest-session teardown is opt-in via `AVISO_E2E_TEARDOWN=1` so successive `uv run pytest` invocations skip the docker startup cost; tests will leave the stack running otherwise.

## Run multiple shards in parallel

To run multiple instances side by side (the existing pattern from before the suite landed), override the host ports and the compose project name. The commands below run from the repo root via `-f`:

```bash
AVISO_SERVER_HOST_PORT=8101 AUTH_O_TRON_HOST_PORT=8181 NATS_HOST_PORT=4322 NATS_MONITORING_HOST_PORT=8322 \
  docker compose -f tests/e2e/docker-compose.yml -p shard-1 up -d
AVISO_SERVER_HOST_PORT=8102 AUTH_O_TRON_HOST_PORT=8182 NATS_HOST_PORT=4323 NATS_MONITORING_HOST_PORT=8323 \
  docker compose -f tests/e2e/docker-compose.yml -p shard-2 up -d
```

`container_name` is intentionally not set so multiple compose projects do not collide on the daemon. The pytest fixture in `tests/e2e/python/conftest.py` uses one stack at session scope and does not exercise this pattern; it is documented here for human operators.

## Update the pinned versions

1. Decide which tag to pin to for the service.
2. Fetch the current manifest-index digest for that tag:

   ```bash
   docker buildx imagetools inspect eccr.ecmwf.int/auth-o-tron/auth-o-tron:<TAG>
   docker buildx imagetools inspect nats:<TAG>-alpine
   docker buildx imagetools inspect eccr.ecmwf.int/aviso/aviso_server:<TAG>
   ```

   Use the top-level `Digest:` value (the multi-arch OCI index digest), not a per-platform manifest digest. Pinning the index keeps the image portable across `linux/amd64` and `linux/arm64`.
3. Update the `image:` line in [`docker-compose.yml`](./docker-compose.yml) and the reference block at the top of this file to `<TAG>@<DIGEST>`.
4. Run the e2e suite locally to confirm.
5. Open a PR with the bump in its own commit.

## Customising the test stack

- [`aviso-server.config.yaml`](./aviso-server.config.yaml) holds the aviso-server side: `auth.mode: direct` pointing at `http://auth-o-tron:8080`, JetStream pointing at `nats://nats:4222`, `connection_max_duration_sec: 15` to keep reconnect tests fast, and the two `notification_schema` entries both with `auth.required: true` and `write_roles: ["producer"]`. Edit for additional streams or alternate auth configurations.
- [`auth-o-tron.config.yaml`](./auth-o-tron.config.yaml) holds the user list and role mappings. Add accounts here when you add tests that need a fresh principal.
- [`nats.conf`](./nats.conf) configures JetStream limits and exposes the monitoring server on `8222`. The aviso-server side picks the stream-level `max_messages` and `discard_policy` via its own `notification_backend.jetstream.*` block.

## CI

CI validates this file with `docker compose config --quiet` so misconfigurations are caught early. The full e2e suite also runs in CI on self-hosted runners (with sccache and a persistent docker image cache); it is informational for now. Promoting it to a required check is tracked in [`plans/roadmap.md`](../../plans/roadmap.md).

## Registry access

`eccr.ecmwf.int` is ECMWF's container registry. GitHub-hosted runners and most public hosts pull from it without credentials for the images we use; if you operate behind a firewall that requires authenticated pulls, configure the appropriate `docker login` step. The `nats` image is on Docker Hub.
