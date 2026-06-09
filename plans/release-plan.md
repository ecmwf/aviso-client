# Release plan — unified 2.0.0

Working plan for the first published release of aviso-client. Living document:
update it as decisions land; move shipped items into
[`progress.md`](./progress.md) and the forward-looking summary in
[`roadmap.md`](./roadmap.md) once a release goes out.

**Resuming on a fresh machine?** Jump to
[§12 (Implementation order)](#12-implementation-order): it carries the current
status, the one-time tool setup, and the exact next task.

Cross-references: the release intent is sketched in
[`roadmap.md`](./roadmap.md) ("What's next → Release", "Prebuilt C++ artifacts").
Reference implementations studied while drafting this plan:

- `aviso-server` `.github/workflows/release.yaml` + `publish-crate.yaml`
  (ordered crates.io publish, tag==version verification, index-propagation
  retry, multi-arch Docker + GitHub Release).
- `tensogram` `.github/workflows/release-preflight.yml`, `publish-crates.yml`,
  `publish-pypi.yml`, `publish-ffi.yml` (the **dry-run preflight** model,
  idempotent sparse-index-poll publishing, TestPyPI toggle, cargo-c FFI
  packaging). These live outside this repo; the patterns are transcribed below
  so this plan stands alone.

---

## 1. Goal and locked decisions

**Goal.** One repeatable, mostly-automated release that publishes the whole
suite under a **single shared version**, with **dry-runs that exercise every
irreversible step before any immutable artifact is created** (crates.io and PyPI
versions cannot be overwritten or deleted-and-reused).

**Decisions locked with the maintainer:**

| # | Decision | Choice |
|---|----------|--------|
| D-a | Version reconciliation | **Unify everything at 2.0.0.** The Rust crates jump 0.1.0 → 2.0.0 so crates.io and PyPI share one version line. 2.0.0 also continues the legacy `pyaviso` PyPI line (the reason it is not 0.1.0). |
| D-b | Bump + release mechanics | **justfile + `cargo-release`.** A `just` recipe drives the local bump/tag; CI does all real publishing. |
| D-c | First-release scope | **Everything, including prebuilt C++ libraries** (crates.io + PyPI + GitHub Release + docs + `libaviso_ffi` artifacts). |
| D-d | crates.io scope | **Publish all four crates:** `finesse`, `aviso`, `aviso-cli`, `aviso-ffi`. |

Open questions that still gate parts of the design are in [§9](#9-open-questions).

---

## 2. Version model — one number for everything

**Single source of truth:** `[workspace.package] version` in the root
`Cargo.toml`.

- All five crates already inherit via `version.workspace = true`, so one edit
  moves `aviso`, `aviso-cli`, `aviso-ffi`, `aviso-py`, `finesse` together.
- `pyaviso` (the PyPI distribution) declares `dynamic = ["version"]`; maturin
  reads the version from the `aviso-py` crate, i.e. the workspace version. So
  PyPI inherits automatically — **no separate Python version field to bump.**
- Because the unified number is **2.0.0**, both registries read `2.0.0` and the
  legacy `pyaviso` line is honored without decoupling.

**Lockstep hazards the bump must handle** (cargo-release does these; a
hand-rolled bump must not forget them):

1. Literal internal pins `version = "=0.1.0"` in
   `crates/aviso-cli/Cargo.toml`, `crates/aviso-ffi/Cargo.toml`,
   `crates/aviso-py/Cargo.toml` (the latter pins both `aviso` and `aviso-cli`)
   must become `"=2.0.0"`. A stale pin makes `cargo publish` resolve the wrong
   dependency or fail.
2. `[workspace.dependencies] finesse = { version = "0.1", path = ... }` must
   track to `"2.0"` (or whatever Q1 decides).
3. `Cargo.lock` must be regenerated — CI enforces `git diff --exit-code
   Cargo.lock`.

---

## 3. Release surface (what ships, and where)

| Artifact | Target | Publish? | Notes |
|----------|--------|----------|-------|
| `finesse` crate | crates.io | ✅ | Generic WHATWG SSE parser. **Must publish first** — public dependency of `aviso`. |
| `aviso` crate | crates.io | ✅ | Core library. Depends on `finesse`. |
| `aviso-cli` crate | crates.io | ✅ | `aviso` binary. Pins `aviso = "=x.y.z"`. |
| `aviso-ffi` crate | crates.io | ✅ | C ABI source crate. Pins `aviso = "=x.y.z"`. |
| `aviso-py` crate | — | ❌ | Builds the wheel; **must get `publish = false`** (see §4). |
| `pyaviso` wheels + sdist | PyPI | ✅ | maturin; manylinux x86_64+aarch64, macOS universal2, abi3, sdist. |
| `libaviso_ffi.{a,so,dylib}` + `aviso.h` + `aviso.hpp` | GitHub Release assets | ✅ | Prebuilt C/C++ libraries (crates.io ships source only). |
| `aviso` CLI binaries | GitHub Release assets | ❓ | Q4 — optional. |
| mdBook docs | ECMWF Sites | ✅ (already) | `docs-sites.yml` already publishes on tags; needs `stable` restricted to semver tags (§6.F). |

**crates.io publish order** (dependency topological):
`finesse` → `aviso` → { `aviso-cli`, `aviso-ffi` } (last two are independent of
each other and may run in parallel).

---

## 4. Pre-release repo cleanups (their own small PR, before any release)

These are prerequisites, not part of the release run itself:

1. **`crates/aviso-py/Cargo.toml`: add `publish = false`.** It builds the
   Python extension; it is never a crates.io crate. `tests/e2e/rust` already has
   this; `aviso-py` does not. Without it, a careless `cargo publish --workspace`
   could try to push it.
2. **`finesse` requirement style: DECIDED `^2` (float).** `cargo-release`
   `dependent-version = "upgrade"` will rewrite `[workspace.dependencies]
   finesse = { version = "0.1", ... }` to `"2.0"` (semantically the caret range
   we want). Do **not** write checks that require the literal text `^2`.
3. **Add the `cargo-release` config** (`release.toml` or
   `[workspace.metadata.release]`): `shared-version = true`,
   `tag-name = "{{version}}"` (bare, per Q2), `publish = false`,
   `push = false`, `dependent-version = "upgrade"`, ordered member list.
4. **Add the `justfile`** (recipes in §5).
5. **Fix crate README links before first publish** (immutable on crates.io): the
   `aviso` and `aviso-ffi` READMEs link `aviso-py` as if it were a crates.io
   crate, but it is not published. Correct those references.
6. **Audit the e2e-crate internal pin:** `tests/e2e/rust/Cargo.toml` also pins
   `aviso = { version = "=0.1.0" }`. It is `publish = false`, but it is a
   workspace member, so the bump and the version-consistency check must cover it.
7. **README/docs copyright note** currently says "Copyright 2026"; confirm the
   year is intended before a public release.

---

## 5. `justfile` (developer-facing; prepares and tags, never publishes)

Division of labor: **the justfile prepares + tags locally; CI publishes.**
Credentials never touch a laptop; a pushed tag is the single trigger.

Proposed recipes (names tentative):

- `just release-preflight 2.0.0`
  Local mirror of the CI preflight (§6.A): version-consistency check across all
  manifests, `cargo package --list` for all four crates, `cargo publish
  --dry-run` in order, `maturin build` wheel + sdist, `twine check`. Fast
  feedback before touching CI.
- `just release-version 2.0.0`
  `cargo release version --workspace --execute 2.0.0` — bumps the workspace
  version, rewrites the literal `=x.y.z` internal pins, updates `Cargo.lock`.
  **`--execute` is required: cargo-release dry-runs by default** and would
  otherwise change nothing. **No tag, no push.** Operator reviews the diff.
- `just release-tag`
  Creates the bare annotated tag `2.0.0` (per Q2) and prints the exact
  `git push origin 2.0.0` command (not `--follow-tags`, which can push unrelated
  annotated tags) rather than pushing automatically.
- `just publish-dry`
  Convenience wrapper to launch the CI dry-run paths (`workflow_dispatch` with
  dry-run inputs) via `gh workflow run`.

`cargo-release` config keys that matter: `shared-version = true` (one version
across the workspace), `tag-name`, `pre-release-commit-message`, ordered
`[[package]]`/members, `publish = false`, `push = false`,
`dependent-version = "upgrade"` (rewrites the `=x.y.z` pins).

---

## 6. CI workflows (all new — none exist in aviso-client today)

Existing workflows: `ci.yml`, `ci-image.yml`, `docs-sites.yml`. The release
ones below are additive. Every workflow that publishes verifies **tag == workspace
version** before doing anything (the aviso-server pattern), and every publisher
is **idempotent** so a re-run after a partial failure is safe.

### 6.A `release-preflight.yml` — the dry-run gate (run BEFORE tagging)

Trigger: `workflow_dispatch(version)`. Publishes nothing, tags nothing. Green =
safe to cut the release. Steps (from tensogram's `release-preflight.yml`):

- **Version consistency.** A `check_version` loop comparing the input version
  against `Cargo.toml [workspace.package]` and any non-inheriting manifest.
  Collect all mismatches, fail once with a count. `pyproject.toml` is **not**
  parsed (its version is `dynamic`); the Python version is validated from the
  built wheel/sdist metadata instead (§6.C). This is the literal enforcement of
  "same version for everything".
- **Quality gate:** `cargo fmt --check`, `cargo clippy -D warnings`,
  `cargo test --workspace`.
- **Tarball assembly:** `cargo package -p <crate> --list` for `finesse`,
  `aviso`, `aviso-cli`, `aviso-ffi` — proves each crate packages (catches
  missing/excluded files) without uploading.
- **crates.io dry-run:** `cargo publish --dry-run` for each crate, in order,
  with a placeholder token.
- **Python:** build wheel + sdist with maturin, then `twine check dist/*` to
  validate PyPI metadata offline. Optionally upload to **TestPyPI**.
- **C++:** build `libaviso_ffi`, regenerate and diff `aviso.h` (header drift
  guard, already in `ci.yml`), build + run a C++ smoke example against the
  staged library.

Q10: keep this manual-only, or also auto-run on PRs that touch version/manifests.

### 6.B `publish-crates.yml` — ordered, idempotent crates.io publish

Triggers: tag push **and** `workflow_dispatch(dry_run: bool, default true)`.
`environment: crates-io` (enables an approval gate / scoped secret). Secret:
`CARGO_REGISTRY_TOKEN`.

Mechanism (tensogram's helper, preferred over aviso-server's grep-on-error):

- **Release-start guard (fail loud):** before publishing anything, assert the
  target `2.0.0` does **not** already exist on the index for any of the four
  crates. A pre-existing `2.0.0` at release start is an error, not a no-op —
  `skip-if-indexed` must NOT be the default path (see §13 B6).
- A `publish_crate` helper that:
  1. Reads the crate version from `cargo metadata`.
  2. Computes the **sparse-index URL** and publishes.
  3. Then **polls the sparse index** (up to ~60s) until the new version appears
     before the next crate — deterministic fix for index-propagation lag between
     dependent publishes.
- **`skip-if-indexed` only in an explicit retry mode**, and only after verifying
  the indexed crate is the artifact produced from *this* tag (checksum), so a
  re-run after a partial failure is safe without masking a divergent upload.
- One step per crate in dependency order: `finesse` → `aviso` →
  `aviso-cli` → `aviso-ffi`.
- Dry-run path: `cargo publish --dry-run` for each, no token needed. **Caveat:**
  for a *first* publish, the dependent dry-runs (`aviso`, `aviso-cli`,
  `aviso-ffi`) cannot fully succeed until their upstreams exist on crates.io —
  see §13 blocking issue B2 and the RC-rehearsal recommendation.

### 6.C `publish-pypi.yml` — wheels + sdist with a TestPyPI lever

Triggers: tag push **and** `workflow_dispatch(use_test_pypi: bool)`.
`environment: ${{ inputs.use_test_pypi && 'test-pypi' || 'pypi' }}`.

- **Build matrix** (per roadmap): manylinux **x86_64** + **aarch64** (maturin in
  `quay.io/pypa/manylinux_2_28_*` containers), **macOS universal2**, **abi3**
  (the crate already sets `abi3-py310`), plus an **sdist**. No Windows, no
  musllinux (roadmap follow-up).
- Upload each as a build artifact; a `publish` job downloads, runs
  `twine check dist/*`, then uploads with `pypa/gh-action-pypi-publish`.
- **`skip-existing` is unsafe for the final upload.** PyPI files are immutable;
  if one bad wheel landed and a re-run uses `skip-existing`, the job skips the
  bad file and fills in the rest, leaving a mixed immutable release. Use
  `skip-existing` only on the **TestPyPI** path. For production, either do not
  skip, or skip only after comparing existing filenames + hashes to the
  artifacts built from this tag (§13 B5).
- **Add a clean-env sdist install test** (not just `twine check`, which only
  validates metadata): in a fresh venv, `pip install dist/pyaviso-2.0.0.tar.gz`,
  then `python -c "import pyaviso"` and `aviso --version`. Document that the
  sdist source-build needs a Rust toolchain (§13 R4).
- **Do not version-check `pyproject.toml`** — it is `dynamic = ["version"]`, so
  there is no static field. Validate the built wheel/sdist **filename + metadata**
  contain `2.0.0`, and confirm maturin resolves it from `aviso-py` via the
  workspace version (§13 R3).
- **Auth: DECIDED OIDC trusted publishing** (no stored secret), API token only as
  a project-scoped fallback in the `pypi` environment (§13 R8; was Q8).

### 6.D `release-cpp-artifacts.yml` — prebuilt C/C++ libraries

Triggers: tag push **and** `workflow_dispatch(version)` (re-attach to an existing
tag). Pattern from tensogram's `publish-ffi.yml`:

- `resolve-ref` job pins ref + version once.
- Per-platform build jobs (Linux x86_64, Linux aarch64, macOS): **stage install
  → smoke-test the packaged artifact → pack a tarball → upload artifact**.
- `release` job downloads all tarballs and attaches them to the GitHub Release.
- Bundle `libaviso_ffi.{a,so,dylib}` with the generated `aviso.h` and the
  hand-written `aviso.hpp`, in the layout a CMake consumer points
  `AVISO_FFI_INCLUDE_DIR` / `AVISO_FFI_LIB_DIR` at.
- **Tooling: DECIDED `cargo-c`** (Q7). Migrate `aviso-ffi` to `cargo cinstall`
  for a standard `.so/.a/.pc/.h` install layout plus the build→smoke→pack
  pattern, replacing the current hand-rolled `cbindgen` + `crate-type` packaging.
  (The `cbindgen` header-drift guard in `ci.yml` stays as a generation check.)
- **Real-stack C++ gate first: DECIDED (Q3, §13 B7).** Today the real-stack
  e2e job is informational and the C++ example only runs `schema_smoke` (no
  server). Before 2.0.0, make the C++ examples run against the real e2e stack and
  gate the release on it; the preflight runs the **packaged** artifact (not just
  the no-server smoke). This lands as its own piece of work **before** the
  prebuilt-C++ artifact workflow.

### 6.E `release.yml` — the GitHub Release

Trigger: tag push. Verifies tag == version, then `softprops/action-gh-release`
with `generate_release_notes: true`, title `aviso-client 2.0.0`.

- **Cross-workflow `needs:` does NOT exist (§13 B8).** GitHub Actions `needs`
  only links jobs *within one workflow*. So the GitHub Release creation and the
  C++ asset attachment **must live in the same workflow** (merge §6.D's `release`
  job and §6.E into one file), or be sequenced with `workflow_run`. Otherwise
  release creation and asset upload race. **Resolution: fold §6.E into
  `release-cpp-artifacts.yml`** as its final job (create release → attach assets
  in one run).

### 6.F Docs — restrict `stable` to release tags (§13 R5)

`docs-sites.yml` currently builds on `tags: ["*"]` and softlinks any tag push as
`stable`. Constrain the canonical/`stable` publish to **semver release tags**
(e.g. `2.0.0`) so an arbitrary tag cannot overwrite the stable docs.

---

## 7. Dry-run / safety architecture (why immutable releases stay safe)

```
  release-preflight.yml  (workflow_dispatch, NO tag, NO publish)
    ├─ version-consistency across all manifests
    ├─ fmt / clippy / test
    ├─ cargo package --list   (all 4 crates)
    ├─ cargo publish --dry-run (all 4, ordered)
    ├─ build wheels + sdist → twine check  [→ optional TestPyPI]
    └─ build + smoke-test C++ libs
           │ green ⇒ safe to release
           ▼
  just release-version 2.0.0 → review → PR → merge → just release-tag → push tag
           │  (tag triggers, in parallel — each gated by the release invariant §13:)
           ├─ publish-crates.yml   (fail-loud-if-indexed, publish, index-poll, ordered)
           ├─ publish-pypi.yml     (twine check + sdist install test; NO skip-existing on prod)
           └─ release-cpp-artifacts.yml (build→smoke→pack→attach→create Release+notes)
                                     docs-sites.yml (stable softlink, semver tags only)
```

**Two independent dry-run layers, both available before any immutable artifact:**

1. **Preflight (pre-tag):** everything except the irreversible upload — but see
   §13 B2: dependent-crate dry-runs cannot fully simulate a *first* publish
   before upstreams exist on crates.io. The honest pre-tag rehearsal is a public
   `2.0.0-rc.1` (§13).
2. **Per-publisher dry-run (`workflow_dispatch`):** `cargo publish --dry-run`,
   `use_test_pypi=true`, build-only C++ — each publisher exercisable alone.

**Safe re-runs, not blind idempotency (§13 B5/B6):** a re-run after a partial
failure must verify that anything already published matches *this* tag's
artifacts (checksums) before skipping it. `skip-existing`/`skip-if-indexed` are
TestPyPI / explicit-retry-mode only, never the default production path.

---

## 8. Release runbook (once the above is built)

1. `just release-preflight 2.0.0` — local dry-run green.
2. Run **`release-preflight.yml`** in CI for `2.0.0` — green.
3. `just release-version 2.0.0` — review the diff (version, the `=x.y.z` pins in
   `aviso-cli`/`aviso-ffi`/`aviso-py`/`tests/e2e/rust`, `Cargo.lock`).
4. Open PR "Release 2.0.0"; land through normal CI (must merge to `main`).
5. **RC rehearsal (first release): publish `2.0.0-rc.1`** end-to-end (real
   crates.io + real PyPI under the reclaimed name + GitHub assets) to prove
   names, owners, auth, ordering, index propagation, and trusted publishing.
   This burns an RC version publicly but never the final `2.0.0` (§13 R1).
6. `just release-tag` → `git push origin 2.0.0`.
7. Tag triggers the publishers; **each first verifies the release invariant**
   (tag == workspace version, tag commit reachable from `origin/main`, `ci-pass`
   green for that exact SHA — §13 B3). Watch them.
8. Verify: crate pages live; a clean-env `pip install pyaviso==2.0.0` imports and
   `aviso --version` works on each target platform; GitHub Release has C++
   tarballs + notes; docs `stable` points at 2.0.0.
9. On partial failure, **do not move the `2.0.0` tag** — follow the recovery
   strategy (§13 R7): stop, yank if needed, bump the whole workspace to `2.0.1`,
   release from a new tag.
10. Update `progress.md` and `roadmap.md` (move "Release" next → shipped).

---

## 9. Open questions

| # | Question | Default / lean |
|---|----------|----------------|
| Q1 | `finesse` requirement style: `=2.0.0` lockstep vs `^2` float? | **DECIDED: `^2` float** (it is a generic parser; minor/patch finesse releases flow without lockstep churn). |
| Q2 | Tag style: bare `2.0.0` vs `v2.0.0`? | **DECIDED: bare `2.0.0`** (matches the tensogram precedent and the legacy `ecmwf/aviso` tags). cargo-release `tag-name = "{{version}}"`; workflows trigger/verify on the bare form. |
| Q3 | Ship prebuilt C++ libs in 2.0.0 *without* the real-stack C++ e2e gate? | **DECIDED: land the real-stack C++ gate first.** Make the C++ examples run against the real e2e stack (not just no-server `schema_smoke`) and gate the release on it before attaching prebuilt libs. |
| Q4 | Also attach prebuilt `aviso` CLI binaries (Linux/macOS) to the Release? | **DECIDED: no.** CLI ships via `cargo install aviso-cli` and the PyPI wheel (which bundles the CLI). No standalone-binary build matrix. |
| Q5 | Changelog: GitHub auto-notes vs a maintained `CHANGELOG.md`? | Lean auto-notes for first cut. |
| Q6 | Full `2.0.0-rc.1` rehearsal before real 2.0.0? | **DECIDED: yes** (§13 R1) for the first release. |
| Q7 | FFI packaging: adopt `cargo-c` vs keep hand-rolled `cbindgen` + pack script? | **DECIDED: adopt `cargo-c`** (standard `.so/.a/.pc/.h` layout + smoke-test pattern). Migrate `aviso-ffi` from the current cbindgen + `crate-type` setup. |
| Q8 | PyPI auth: OIDC trusted publishing vs API token? | **DECIDED: OIDC trusted publishing**; API token only as a project-scoped fallback in the `pypi` environment. |
| Q9 | crates.io index race: sparse-index poll (tensogram) vs retry-on-error (aviso-server)? | **DECIDED: sparse-index poll.** |
| Q10 | Preflight trigger: manual-only vs also auto on version/manifest PRs? | **DECIDED: manual-only** (`workflow_dispatch`). |

---

## 10. External / out-of-band prerequisites (humans + consoles, not code)

These block a real publish and must be done in the respective accounts:

- **crates.io:** own/claim the names `finesse`, `aviso`, `aviso-cli`,
  `aviso-ffi`; add `CARGO_REGISTRY_TOKEN` repo/environment secret. First publish
  of each name is irreversible — verify availability early (name-squat risk).
- **PyPI:** the team must have owner rights on the existing **`pyaviso`**
  project; configure **trusted publishing** (or mint an API token, Q8); set up a
  `pypi` and `test-pypi` GitHub Environment. **Archive the old `pyaviso`** with a
  deprecation README pointing here (roadmap commitment).
- **GitHub:** branch protection requiring `ci-pass`; `crates-io` / `pypi`
  environments with required reviewers if a manual approval gate is wanted;
  confirm `contents: write` for the release job.
- **ECMWF Sites:** already wired (`docs-sites.yml`); confirm `stable` softlink
  behavior on first tag is desired.
- **Self-hosted runners:** confirm the C++ cross-build matrix has the needed
  runners/targets (Linux aarch64, macOS arm64).

---

## 11. Cross-repo prerequisite: legacy `pyaviso` deprecation (DONE)

The "additional steps" tracked here were the legacy `ecmwf/aviso` deprecation —
a prerequisite for this release because the new client reclaims the `pyaviso`
PyPI name at 2.0.0 and continues that version line. Completed:

- Deprecation notice in the legacy README + docs, pointing to `aviso-client` /
  `aviso-server` and their docs sites (ecmwf/aviso#31, merged).
- Read the Docs build repaired (added `.readthedocs.yaml`; version read without
  importing the package).
- `pyaviso 1.0.2` published to PyPI so the deprecation notice is the live PyPI
  project description **before** the 2.0.0 client takes the name.
- **Pending (admin-side, non-blocking):** Read the Docs `latest` repointed from
  the retired `develop` branch to `main`; then delete the stopgap `develop`
  branch. No code dependency on this for the aviso-client release.

With the legacy deprecation shipped, this cross-repo prerequisite is satisfied
and the aviso-client release is unblocked.

---

## 12. Implementation order

Recorded so we can resume after a context reset. Ordered so workflow-shaping
questions and external prereqs are resolved *before* the workflows that depend
on them, not at the end.

All shaping questions are now decided (Q1–Q4, Q6–Q10); only Q5 (changelog) is
still open.

**Where we are:** steps 2-4 are merged to `main`; step 5 is partway through a
staged migration. Done: foundation (#48), release-invariant check (#50), the
real-stack C++ e2e gate now in `ci-pass` (#51), and the cargo-c packaging
foundation (#52, "PR-B0" below). **The next task is step 5 PR-A** (bake
`pkg-config` + `cargo-c` into the pinned CI image). Step 1 (external prereqs) is
console work that can run in parallel; steps 6-10 are not started.

**Step 5 is staged across several PRs** (an Oracle-reviewed sequence, to keep
every existing consumer green while cargo-c becomes an additional packaging
path, not yet a replacement):

- **PR-B0 — DONE (#52).** Additive `[package.metadata.capi]` + `capi` feature on
  `aviso-ffi` (header generation off, so the committed drift-guarded `aviso.h`
  stays the source of truth), `just ffi-cinstall`, and a GitHub-hosted
  `ffi-cargo-c` ci-pass job that `cargo cinstall`s to a staging prefix and
  builds a pkg-config consumer against it. `crate-type` kept; the existing
  `cpp`/`e2e` jobs are untouched. Verified the install layout:
  `include/aviso_ffi/{aviso.h,aviso.hpp}`, `lib/libaviso_ffi.{a,so,dylib}`,
  `lib/pkgconfig/aviso_ffi.pc`.
- **PR-A — NEXT.** Add `pkg-config` + `cargo-c` to `.github/ci/Dockerfile`, bump
  `.github/ci/VERSION` (0.3.0 → 0.4.0), republish via `ci-image.yml` to the ECMWF
  registry (needs registry creds / maintainer console), then bump the `image:`
  tag in `ci.yml`. This is the critical-path blocker for moving cargo-c onto the
  pinned image.
- **PR-B1.** Move the `ffi-cargo-c` smoke onto the pinned image; drop the
  GitHub-hosted bootstrap job.
- **PR-C (≈ step 9).** `release-cpp-artifacts.yml` uses `cargo cinstall` to
  package the per-platform tarball; optionally migrate the in-tree `examples/cpp`
  fully to pkg-config and retire the hand-rolled `find_library` path.

**One-time tool setup (per machine):** `cargo install just cargo-release`, and
install [`uv`](https://docs.astral.sh/uv/) for the Python preflight steps
(`cargo-c` is only needed from step 5). Then `just --list` and continue below.

1. **External prereqs in flight early (§10):** crates.io name ownership +
   `CARGO_REGISTRY_TOKEN`; PyPI owner rights + OIDC trusted-publishing config +
   `pypi`/`test-pypi` environments; branch protection. First-publish names are
   irreversible — start these before writing publishers.
2. **Foundation (§4) — DONE (#48).** `publish = false` on `aviso-py`;
   `cargo-release` config (`release.toml`); `justfile`; crate README links fixed;
   `tests/e2e/rust` pin audited. No version change, no publish.
3. **Release-invariant composite action / reusable check (§13 B3) — DONE (#50).**
   `.github/actions/release-invariant` + `scripts/release-invariant.sh` assert
   tag == workspace version, tag commit reachable from `origin/main`, and
   `ci-pass` green for the SHA. Every publisher calls it first.
4. **Real-stack C++ e2e gate (Q3) — DONE (#40 ran the C++ examples on the real
   stack; #51 promoted the `e2e` job to `ci-pass` with bring-up hardening).**
5. **`aviso-ffi` → `cargo-c` migration (Q7) — IN PROGRESS.** PR-B0 done (#52);
   PR-A (CI image) is the next task. See the staged breakdown under "Where we
   are" above. Keep the `cbindgen` header-drift guard (it stays the header source
   of truth; cargo-c installs the committed header, generation off).
6. `release-preflight.yml` (§6.A) — dry-run gate; validate on the current 0.1.0
   tree before any bump. Includes the clean-env sdist install test and the
   real-stack packaged C++ artifact run.
7. `publish-crates.yml` (§6.B) with fail-loud-if-indexed + dry-run path.
8. `publish-pypi.yml` (§6.C) with TestPyPI path (OIDC); validate via TestPyPI.
9. `release-cpp-artifacts.yml` (§6.D, cargo-c) **with the GitHub Release creation
   folded in** (§6.E); restrict `docs-sites.yml` `stable` to semver tags (§6.F).
10. **RC rehearsal `2.0.0-rc.1`** end-to-end (§13 R1), then the real `2.0.0`.

---

## 13. Release safety requirements (incorporated)

Folded into the sections above; recorded here as the authoritative checklist so
nothing is lost.

### Blocking issues (must be true before/within the workflows)

- **B1 — `cargo release version` needs `--execute`.** It dry-runs by default and
  changes nothing otherwise. Fixed in §5.
- **B2 — first-publish dry-run is not fully provable.** `cargo publish --dry-run`
  for `aviso`/`aviso-cli`/`aviso-ffi` cannot succeed until their upstreams exist
  on crates.io (published manifests drop `path` and resolve from the registry).
  So "all four ordered dry-run green before any immutable artifact" is not
  achievable for a first release. Honest mitigations: a public `2.0.0-rc.1`
  rehearsal (R1), and `--no-verify` packaging checks where appropriate.
- **B3 — tag-triggered publish bypasses branch protection.** Branch protection on
  `main` does not stop a bare `2.0.0` tag pushed at an unreviewed commit. **Every
  publisher must first assert the release invariant:** (1) tag name == workspace
  version, (2) the tag commit is reachable from `origin/main`, (3) `ci-pass`
  succeeded for that exact SHA. Implement once as a reusable check (§12 step 3).
- **B4 — the e2e crate has an internal pin too.** `tests/e2e/rust/Cargo.toml`
  pins `aviso = "=0.1.0"`; include it in the bump + version-consistency check
  even though it is `publish = false`. Fixed in §4.6.
- **B5 — PyPI `skip-existing` can launder a partial bad upload** into a false
  green. Prod path: no `skip-existing` (or hash-compare first). Fixed in §6.C.
- **B6 — crates.io `skip-if-indexed` is unsafe as the default.** A pre-existing
  target `2.0.0` at release start is an error, not success. Skip only in an
  explicit retry mode after checksum-verifying the indexed artifact. Fixed in §6.B.
- **B7 — prebuilt C++ without a real-stack gate is a real risk.** The current C++
  example is no-server `schema_smoke`; the real-stack e2e is informational. If
  C++ libs ship in 2.0.0, the preflight must run the packaged artifact against
  the real stack. Fixed in §6.D (and Q3).
- **B8 — cross-workflow `needs:` does not exist.** Fold the GitHub Release
  creation into `release-cpp-artifacts.yml` (or use `workflow_run`). Fixed in §6.E.

### Strong recommendations (adopted)

- **R1 — public `2.0.0-rc.1` rehearsal** is the only honest way to exercise every
  irreversible step (names, owners, auth, ordered publish, index propagation,
  PyPI trusted publishing, GitHub assets) before final `2.0.0`. crates.io has no
  TestPyPI equivalent. Added to §8/§12.
- **R3 — do not parse `pyproject.toml` for the Python version** (it is
  `dynamic`); validate the built wheel/sdist filename + metadata instead. §6.C.
- **R4 — clean-env sdist install test** (`pip install` the sdist, `import
  pyaviso`, `aviso --version`), not just `twine check`. §6.C.
- **R5 — restrict `docs-sites.yml` `stable` to semver tags.** §6.F.
- **R6 — fix `aviso`/`aviso-ffi` README links** that reference `aviso-py` as a
  crates.io crate before first publish (immutable first impression). §4.5.
- **R7 — partial-publish recovery:** never move the `2.0.0` tag to republish
  mixed sources. Stop, yank if appropriate, bump the whole workspace to `2.0.1`,
  release from a new tag. §8 step 9.
- **R8 — OIDC trusted publishing** for PyPI; token only as project-scoped
  fallback. Decided (was Q8).

### Nits (adopted)

- Removed stale `tag-name = "v{{version}}"` wording (bare per Q2).
- `git push origin 2.0.0`, not `--follow-tags`.
- `cargo doc --workspace --no-deps` with `RUSTDOCFLAGS=-D warnings` in preflight
  if docs.rs quality matters for the first crates.io release.
- crates.io 404 on the names is **not** a reservation; re-check availability
  immediately before release start.
