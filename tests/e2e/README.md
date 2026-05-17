# End-to-end tests

E2E tests run against a real `aviso-server` instance built from a pinned commit. The pinned SHA lives in [`.env`](./.env) as `AVISO_SERVER_SHA` and is the single source of truth; `docker-compose.yml` reads it via variable substitution. The `.env` file is intentionally checked in — it carries no secrets, only the pinned commit identifier.

## Run locally

```bash
docker compose -f tests/e2e/docker-compose.yml --env-file tests/e2e/.env up --build -d
docker compose -f tests/e2e/docker-compose.yml --env-file tests/e2e/.env logs -f aviso-server  # optional
# … run e2e tests against http://localhost:8000 …
docker compose -f tests/e2e/docker-compose.yml --env-file tests/e2e/.env down
```

Docker compose also auto-loads a `.env` file from its own directory, so `cd tests/e2e && docker compose up --build -d` works without `--env-file`.

## Update the pinned version

1. Decide which `aviso-server` commit to pin to.
2. Update `AVISO_SERVER_SHA` in [`.env`](./.env) to the new full SHA.
3. Run the e2e suite locally to confirm.
4. Open a PR with the SHA bump in its own commit, linking the `aviso-server` change(s) it tracks.

## CI

CI runs the e2e suite as a separate workflow (TBD when Phase 1 actually has e2e tests). Phase 0 ships the harness but does not execute it.
