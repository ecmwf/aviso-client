# End-to-end tests

E2E tests run against a real `aviso-server` instance pulled from ECMWF's container registry, pinned by manifest digest for reproducibility:

```text
eccr.ecmwf.int/aviso/aviso_server:0.6.2@sha256:2c2607d1ba4d9b4bf55e52bf9b08793d8ca1959843a3efaa3d6f6922d5fdc60c
```

The tag (`0.6.2`) is a human-readable label; the digest is what Docker resolves and verifies. A re-published tag cannot silently change CI or local runs, because the digest will no longer match. The harness mounts [`aviso-server.config.yaml`](./aviso-server.config.yaml) into the container as the server's config file (via `AVISOSERVER_CONFIG_FILE`), so the test environment runs unauthenticated, in-memory-only, with short heartbeat and connection-lifetime settings tuned for fast tests.

## Run locally

```bash
cd tests/e2e
docker compose up -d                              # pulls the image
docker compose logs -f aviso-server               # optional
# … run e2e tests against http://localhost:8000 …
docker compose down
```

To run multiple instances in parallel (e.g. CI shards), override the host port and the compose project name:

```bash
AVISO_SERVER_HOST_PORT=8101 docker compose -p shard-1 up -d
AVISO_SERVER_HOST_PORT=8102 docker compose -p shard-2 up -d
```

`container_name` is intentionally not set so multiple compose projects do not collide on the daemon. Readiness is checked by the test harness against `http://127.0.0.1:${AVISO_SERVER_HOST_PORT}/health`; the compose file does not embed a healthcheck because the upstream image may not include `curl`.

## Update the pinned version

1. Decide which `aviso-server` tag to pin to.
2. Fetch the current manifest-index digest for that tag:

   ```bash
   docker buildx imagetools inspect eccr.ecmwf.int/aviso/aviso_server:<TAG>
   ```

   Use the top-level `Digest:` value (the multi-arch OCI index digest), not a per-platform manifest digest. Pinning the index keeps the image portable across `linux/amd64` and `linux/arm64`.
3. Update the `image:` line in [`docker-compose.yml`](./docker-compose.yml) and the reference block at the top of this file to `<TAG>@<DIGEST>`.
4. Run the e2e suite locally to confirm.
5. Open a PR with the bump in its own commit.

## Customising the test server

The mounted [`aviso-server.config.yaml`](./aviso-server.config.yaml) ships with auth disabled, an in-memory backend, a short connection-lifetime to surface reconnect/resume bugs quickly, and two example schemas (`test_event`, `test_polygon`). Edit it for additional streams, or to enable auth and exercise the `auth-o-tron` flow.

## CI

CI validates this file with `docker compose config --quiet` so misconfigurations are caught early. The full e2e suite runs once real tests exist.

## Registry access

`eccr.ecmwf.int` is ECMWF's container registry. GitHub-hosted runners pull from it without credentials for public images; if `aviso_server:0.6.2` is published privately, configure the appropriate `docker login` step in CI or run e2e tests on a self-hosted runner with registry access.
