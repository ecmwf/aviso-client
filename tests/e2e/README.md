# End-to-end tests

E2E tests run against a real `aviso-server` instance built from a pinned commit. The pinned SHA lives in [`.aviso-server-version`](./.aviso-server-version) and is read by [`docker-compose.yml`](./docker-compose.yml).

## Run locally

```bash
docker compose -f tests/e2e/docker-compose.yml up --build -d
docker compose -f tests/e2e/docker-compose.yml logs -f aviso-server  # optional
# … run e2e tests against http://localhost:8000 …
docker compose -f tests/e2e/docker-compose.yml down
```

## Update the pinned version

1. Decide which `aviso-server` commit to pin to.
2. Update [`.aviso-server-version`](./.aviso-server-version) with the new full SHA.
3. Run the e2e suite locally to confirm.
4. Open a PR with the SHA bump in its own commit, linking the `aviso-server` change(s) it tracks.

## CI

CI runs the e2e suite as a separate workflow (TBD when Phase 1 actually has e2e tests). Phase 0 ships the harness but does not execute it.
