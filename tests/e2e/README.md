# End-to-end tests

E2E tests run against a real `aviso-server` instance built from a pinned commit. The pinned SHA lives in [`.env`](./.env) as `AVISO_SERVER_SHA` and is the single source of truth; `docker-compose.yml` reads it via variable substitution. The `.env` file is intentionally checked in — it carries no secrets, only the pinned commit identifier.

## Run locally

```bash
cd tests/e2e
docker compose up --build -d                       # uses .env automatically
docker compose logs -f aviso-server                # optional
# … run e2e tests against http://localhost:8000 …
docker compose down
```

Docker compose auto-loads `.env` from its own directory. To run multiple instances in parallel (e.g. CI shards), override the host port and the project name:

```bash
AVISO_SERVER_HOST_PORT=8101 docker compose -p shard-1 up --build -d
AVISO_SERVER_HOST_PORT=8102 docker compose -p shard-2 up --build -d
```

`container_name` is intentionally not set so multiple compose projects do not collide on the daemon. Readiness is checked by the test harness against `http://127.0.0.1:${AVISO_SERVER_HOST_PORT}/health`; the compose file does not embed a healthcheck because the upstream Dockerfile may not include `curl`.

## Update the pinned version

1. Decide which `aviso-server` commit to pin to.
2. Update `AVISO_SERVER_SHA` in [`.env`](./.env) to the new full SHA.
3. Run the e2e suite locally to confirm.
4. Open a PR with the SHA bump in its own commit, linking the `aviso-server` change(s) it tracks.

## CI

CI validates this file with `docker compose config --quiet` so misconfigurations are caught early. The full e2e suite runs once real tests exist.
