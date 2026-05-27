# E2E CI on self-hosted runners

The e2e integration test suite (`plans/e2e-integration-suite.md`) ships as a locally-runnable suite first; this plan covers wiring it into CI once self-hosted runners are available on the repo.

## Why this is a separate plan

A GitHub-hosted `ubuntu-latest` runner would add ~3-5 min warm and ~6-9 min cold to every PR cycle. The dominant costs are docker image pulls (three images per run, network-bound), cold Rust builds (the e2e crate plus the debug-mode `aviso` CLI binary that `assert_cmd::cargo_bin` resolves), and the reconnect-test floor (~15 s per reconnect cycle). Two of those three costs evaporate on a self-hosted runner with persistent disk:

- Docker images persist between runs; pinned-by-digest images guarantee `docker compose up` is fast unless the digest itself changed.
- The Rust target directory persists on disk; combined with `sccache` against the user's existing S3 bucket, cold builds become incremental even across runners.

Expected warm cycle on a self-hosted runner: ~1-2 min added to CI. Cold cycle (first run after a Cargo.lock or image bump): ~3-5 min.

This plan is on hold pending the user's conversation with their colleague about assigning self-hosted runners to the `ecmwf/aviso-client` repo.

## What the workflow needs

### Runner shape

- One or more self-hosted Linux x86-64 runners registered with the repo (or the ECMWF org with a repo-scoped label).
- Runner labels: a project-specific label like `aviso-e2e` plus the standard `self-hosted, linux, x64`. The workflow's `runs-on:` matches all three.
- Docker daemon installed and reachable by the runner user.
- Persistent disk for: docker image cache (handled by the daemon itself), the runner's `_work` directory (handled by the runner), and the `~/.cargo` + `~/.rustup` + sccache local cache directories (handled by the runner user's home).

### sccache against the existing S3 bucket

The user has an S3 bucket provisioned. Workflow env vars (sourced from repo secrets):

- `RUSTC_WRAPPER=sccache` (signals cargo to route compilations through sccache).
- `SCCACHE_BUCKET=<bucket-name>` (the bucket to cache into).
- `SCCACHE_REGION=<region>` (the bucket's AWS region).
- `SCCACHE_S3_KEY_PREFIX=aviso-client/` (so other repos sharing the same bucket do not collide).
- `AWS_ACCESS_KEY_ID` / `AWS_SECRET_ACCESS_KEY`: scoped IAM credentials with `s3:GetObject` and `s3:PutObject` on the prefix above. Stored in repo secrets, not org-wide.

The sccache binary is installed via `mozilla-actions/sccache-action@v0.0.6` (or whichever stable release is current when the work starts; AGENTS.md's "latest stable" rule applies). The action handles the install, the version, and the cache-stats summary on each run.

### Docker image cache

No `actions/cache` plumbing needed: on a self-hosted runner, docker pulls populate the local daemon's image store and stay there until manually pruned. Pinned-by-digest images mean a fresh `docker compose up` re-uses the cached layers byte-for-byte unless the digest changed. The bump-procedure documented in `tests/e2e/README.md` (manually editing the `<tag>@<digest>` reference) is the only event that forces a fresh pull.

A periodic `docker system prune --filter "until=168h"` cron on the runner host keeps disk usage in check; this is ops policy on the runner box, not part of the workflow.

### Workflow shape

A new job `e2e` is added to `.github/workflows/ci.yml`:

```yaml
e2e:
  name: e2e
  # Self-hosted runners must never execute untrusted code from forks.
  # The Security section below explains the rationale; the job-level guard
  # below enforces the policy on every step.
  if: github.event_name == 'push' || github.event.pull_request.head.repo.full_name == github.repository
  runs-on: [self-hosted, linux, x64, aviso-e2e]
  needs: []
  env:
    RUSTC_WRAPPER: sccache
    SCCACHE_BUCKET: ${{ secrets.SCCACHE_BUCKET }}
    SCCACHE_REGION: ${{ secrets.SCCACHE_REGION }}
    SCCACHE_S3_KEY_PREFIX: aviso-client/
    AWS_ACCESS_KEY_ID: ${{ secrets.SCCACHE_AWS_ACCESS_KEY_ID }}
    AWS_SECRET_ACCESS_KEY: ${{ secrets.SCCACHE_AWS_SECRET_ACCESS_KEY }}
  steps:
    - uses: actions/checkout@v4
    - id: filter
      uses: dorny/paths-filter@v3
      with:
        filters: |
          affected:
            - 'crates/**'
            - 'python/**'
            - 'tests/e2e/**'
            - '.github/workflows/ci.yml'
            - 'Cargo.toml'
            - 'Cargo.lock'
            - 'pyproject.toml'
            - 'uv.lock'
    - if: steps.filter.outputs.affected == 'true'
      uses: mozilla-actions/sccache-action@v0.0.6
    - if: steps.filter.outputs.affected == 'true'
      uses: astral-sh/setup-uv@v4
    - if: steps.filter.outputs.affected == 'true'
      run: uv sync --locked --group dev
    - if: steps.filter.outputs.affected == 'true'
      run: uv run maturin develop --locked
    - if: steps.filter.outputs.affected == 'true'
      run: cargo build -p aviso-cli  # debug build; CLI e2e tests use cargo_bin which expects target/debug/aviso
    - if: steps.filter.outputs.affected == 'true'
      run: bash tests/e2e/shared/stack_up.sh
    - if: steps.filter.outputs.affected == 'true'
      run: uv run pytest tests/e2e/python/ -v
    - if: steps.filter.outputs.affected == 'true'
      run: cargo test --locked -p aviso-e2e -- --include-ignored --test-threads=1
    - if: always() && steps.filter.outputs.affected == 'true'
      run: docker compose -f tests/e2e/docker-compose.yml logs > tests/e2e/last-failure.log
    - if: failure() && steps.filter.outputs.affected == 'true'
      uses: actions/upload-artifact@v4
      with:
        name: e2e-logs
        path: tests/e2e/last-failure.log
    - if: always() && steps.filter.outputs.affected == 'true'
      run: docker compose -f tests/e2e/docker-compose.yml down -v
```

The paths-filter pattern (gating every other step on `steps.filter.outputs.affected == 'true'`) keeps a single required-check name in branch protection without paying CI cost on doc-only PRs. The job exits 0 when the filter says skip.

### Security

GitHub strongly recommends against running self-hosted runners on jobs triggered by fork PRs: a third-party PR could exfiltrate the runner's secrets or persist payloads on the runner's disk. Concrete mitigations:

- The e2e job gates on `github.event.pull_request.head.repo.full_name == github.repository` so fork PRs skip it. Fork PRs from external contributors get the existing GitHub-hosted gates (the hermetic Rust and Python jobs); the e2e gate is added only after a maintainer pushes their work to the canonical repo.
- Document the fork-PR policy in `CONTRIBUTING.md`: external contributors should expect a maintainer to land their changes on a branch in `ecmwf/aviso-client` before the e2e gate runs.
- Repo secrets (sccache S3 creds) are scoped to a single IAM principal with bucket-only access. No org-level secrets are exposed to self-hosted runners.
- The runner's local docker daemon is reachable only from the runner user; no `0.0.0.0` exposure.

### Concurrency

- `concurrency: { group: e2e-${{ github.ref }}, cancel-in-progress: true }` on the workflow file so a force-push to a PR cancels the prior in-flight e2e run. Saves runner time on busy PRs.
- The `tests/e2e/docker-compose.yml` already supports parallel shards via `AVISO_SERVER_HOST_PORT` + `-p <project>` per the README; if two e2e jobs need to share a single runner, the workflow can shard ports based on `github.run_id`. Single-runner-per-job is the default; sharding is a future tuning if contention shows up.

## Roll-out

Three commits in a separate PR after the e2e suite lands on main:

1. **Commit 1: workflow file + paths-filter scaffolding**. Adds the `e2e` job stub to `.github/workflows/ci.yml` with all steps gated on the paths-filter, sccache action wired, fork-PR guard. No tests run yet (the conditional gating prevents any execution until secrets are populated and runners assigned). Verify in a PR that the job appears, gates correctly, and skips on doc-only PRs.

2. **Commit 2: runner registration + secrets, enable execution**. After secrets are populated and runners registered, flip the job from "skip everything" to "run when affected". First real run is on this commit's own PR; if it goes green, the gate is live.

3. **Commit 3: docs**. Update `CONTRIBUTING.md` with the fork-PR policy and the local-vs-CI distinction. Update `README.md` with a sentence pointing at the e2e job's existence. Update `tests/e2e/README.md` to note that the suite now has a CI gate on protected branches.

## Open questions

1. **Runner count and ownership**. How many self-hosted runners? Who owns provisioning, OS patching, docker upgrades, disk space monitoring? User to settle with the colleague.
2. **sccache bucket scope and credentials**. IAM principal for the CI user; bucket lifecycle policy (how long to keep cached objects); cross-repo prefix isolation. To settle when the bucket is wired in.
3. **Fork-PR policy specifics**. The fork-PR guard above is the simplest variant (e2e gate skipped on forks). An alternative (`pull_request_target` with a maintainer-approval label) gives forks faster feedback but is significantly more dangerous. Default to the simpler variant unless the contributor flow demands otherwise.
4. **Image-bump invalidation visibility**. With self-hosted runners, the cache invalidation on a digest bump is invisible (the docker daemon just pulls the new image). The bump PR should document expected runtime delta in the PR description so reviewers know the first run pays the pull cost.

## What stays the same

- `plans/e2e-integration-suite.md` is unchanged by this plan landing; the e2e suite is the same locally-runnable suite, just now also gated in CI.
- The hermetic Rust and Python jobs in CI keep their existing shape.
- The `e2e-config` job (`docker compose config --quiet`) keeps running on the existing GitHub-hosted runners as a cheap pre-check.

## Status

- Not yet planned in detail beyond this sketch.
- Blocked on: self-hosted runner assignment to the repo; S3 bucket access wired via repo secrets.
- Trigger to start: user signals that runners are available and S3 credentials are ready.
