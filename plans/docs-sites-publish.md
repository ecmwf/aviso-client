# Docs publishing to ECMWF Sites

Adds a workflow that builds the mdBook docs and publishes them to ECMWF Sites
at <https://sites.ecmwf.int/docs/aviso-client>. Modelled on the `ecmwf/tensogram`
`docs-sites.yaml` (same reusable workflows, same infra), adapted to this repo's
self-hosted container CI.

No change to `ci.yml`. The existing `docs` job there (`mdbook build` + `mdbook
test`, part of `ci-pass`) stays the merge gate. This workflow is additive: it
builds the publish artifact and pushes it to Sites. Gate and publish stay
separate so a publish flow never entangles the required `ci-pass` check.

## Decisions taken

- **Build inside the CI image, versions from the image.** `build-docs` runs in
  `eccr.ecmwf.int/aviso/cli-ci:0.2.0`, which already bakes `mdbook` and
  `mdbook-mermaid` as pinned static binaries. No `cargo install`, no
  `actions/cache`, no `MDBOOK_VERSION` env vars. The single source of truth for
  the doc tool versions is `.github/ci/Dockerfile`. This is strictly less
  duplication than tensogram's approach (which repeats versions in workflow env
  and relies on a warm cache). Refresh the tools the same way the rest of CI
  does: bump the Dockerfile `ARG` + `.github/ci/VERSION`.
- **No sccache, no S3 creds on the docs build.** `mdbook build docs` renders
  Markdown to HTML and runs the mermaid preprocessor (a prebuilt binary). It
  compiles no Rust, so sccache would have nothing to cache and the S3 creds
  would be dead config and needless secret exposure. This matches the existing
  scoping rule in `ci-self-hosted-sccache.md`: sccache and S3 creds go only to
  the Rust-compiling jobs (`rust`, `python`); `deny` and `docs` never receive
  them. The doctest gate (`mdbook test`, which does compile) already lives in
  `ci.yml` and stays there.
- **Self-hosted, trusted context only.** `build-docs` is gated
  `github.event_name != 'pull_request' || head.repo.full_name == github.repository`
  and skipped on PR close. Fork PRs cannot build here (the image is private and
  the runners are self-hosted), which is consistent with the rest of this
  repo's CI: forks run no self-hosted job and a maintainer lands external work
  on a repo branch to gate it.
- **Least privilege.** Workflow default `permissions: contents: read`.
  `build-docs` keeps `contents: read`. Only `preview-publish` gets
  `pull-requests: write` (for the preview-link comment). The Sites token
  (`ECMWF_SITES_DOCS_AVISOCLIENT_TOKEN`) reaches only the publish/unpublish
  jobs through the reusable-workflow `secrets:` block.
- **Concurrency that never breaks a live publish.** One run per PR or ref.
  `cancel-in-progress` is true only for `pull_request` events, so rapid PR
  pushes cancel their own stale rebuilds, while a push to `main` or a tag never
  cancels an in-flight canonical publish (no half-uploaded site).
- **Injection-safe publish id.** `compute-publish-id` reads `github.ref_name`
  and the manual `publish_id` input through step `env:`, never inline
  `${{ }}` in the shell, then sanitizes to a URL/path-safe slug.
- **Pinned reusable workflows.** `pr-preview-publish.yml`,
  `pr-preview-unpublish.yml`, and `docs-publish.yml` are pinned to the same SHA
  tensogram runs in production (`5a1d1cb1442aa632f7823e4088d639b980627afc`).
  Note: those workflows call their own sub-actions at `@main`, so the SHA pin
  covers the orchestration only, not the leaf actions.

## Sites layout

- Space `docs`, name `aviso-client`.
- Canonical: `path: ""`, id = sanitized ref name. Push to `main` softlinks
  `latest`; a tag softlinks `stable`.
- Preview: `path: pull-requests`, published as `PR-<number>`, removed on close.

## Triggers

- `pull_request: [opened, synchronize, reopened, closed]` (preview lifecycle).
- `push: branches main, tags *` (canonical publish).
- `workflow_dispatch`: builds the artifact only by default; `publish: true`
  opts into a canonical publish, with an optional `publish_id`.

## Verification

- `actionlint` clean (self-hosted labels already declared in
  `.github/actionlint.yaml`).
- First real run validated by a maintainer: PR preview publishes and the link
  comment lands; on merge to `main` the canonical site updates and the `latest`
  softlink points at it.
