# Release plan — unified 2.0.0

Working plan for the first published release of aviso-client. This is a living
document: it is **not complete** — the maintainer has additional steps to add
(see [§10](#10-additional-steps-to-be-filled-in)). Update it as decisions land;
move shipped items into [`progress.md`](./progress.md) and the forward-looking
summary in [`roadmap.md`](./roadmap.md) once a release goes out.

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
| mdBook docs | ECMWF Sites | ✅ (already) | `docs-sites.yml` already publishes on tags and softlinks `stable`; no change needed. |

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
2. **Decide `finesse` requirement style** (Q1) and set it in
   `[workspace.dependencies]`.
3. **Add the `cargo-release` config** (`release.toml` or
   `[workspace.metadata.release]`): `shared-version = true`,
   `tag-name = "v{{version}}"` (or bare, per Q2), `publish = false`,
   `push = false`, ordered member list.
4. **Add the `justfile`** (recipes in §5).
5. **README/docs copyright note** currently says "Copyright 2026"; confirm the
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
  `cargo release version 2.0.0` — bumps the workspace version, rewrites the
  literal `=x.y.z` internal pins, updates `Cargo.lock`. **No tag, no push.**
  Operator reviews the diff.
- `just release-tag`
  Creates the annotated tag (`v2.0.0` or `2.0.0`, per Q2) and prints the
  `git push --follow-tags` command rather than pushing automatically.
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
  against every manifest (`Cargo.toml [workspace.package]`, `pyproject.toml`,
  and any non-inheriting manifest). Collect all mismatches, fail once with a
  count. This is the literal enforcement of "same version for everything".
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

- A `publish_crate` helper that:
  1. Reads the crate version from `cargo metadata`.
  2. Computes the **sparse-index URL** for the crate and **skips if that exact
     version is already published** (idempotent re-runs).
  3. Publishes, then **polls the sparse index** (up to ~60s) until the new
     version appears before the next crate — deterministic fix for
     index-propagation lag between dependent publishes.
- One step per crate in dependency order: `finesse` → `aviso` →
  `aviso-cli` → `aviso-ffi`.
- Dry-run path: `cargo publish --dry-run` for each, no token needed.

### 6.C `publish-pypi.yml` — wheels + sdist with a TestPyPI lever

Triggers: tag push **and** `workflow_dispatch(use_test_pypi: bool)`.
`environment: ${{ use_test_pypi && 'test-pypi' || 'pypi' }}`.

- **Build matrix** (per roadmap): manylinux **x86_64** + **aarch64** (maturin in
  `quay.io/pypa/manylinux_2_28_*` containers), **macOS universal2**, **abi3**
  (the crate already sets `abi3-py310`), plus an **sdist**. No Windows, no
  musllinux (roadmap follow-up).
- Upload each as a build artifact; a `publish` job downloads, runs
  `twine check dist/*`, then uploads with `pypa/gh-action-pypi-publish`
  (`skip-existing: true`).
- **Auth:** OIDC trusted publishing vs API token — Q8.

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
- **Tooling choice (Q7):** adopt `cargo-c` (clean `.pc`/header/lib layout +
  smoke-test pattern) vs keep the current hand-rolled `cbindgen` + `crate-type`
  and write a pack script.
- **Caveat:** the roadmap wanted a real-stack C++ e2e gate to land *before*
  shipping prebuilt libs (Q3).

### 6.E `release.yml` — the GitHub Release

Trigger: tag push. Verifies tag == version, then `softprops/action-gh-release`
with `generate_release_notes: true`, title `aviso-client v2.0.0`, attaching the
C++ tarballs (and optionally CLI binaries, Q4). Depends on the C++ artifact job
for its assets.

### 6.F Docs — no change

`docs-sites.yml` already publishes on tags and softlinks `stable`. The new tag
just works.

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
           │  (tag triggers, in parallel:)
           ├─ publish-crates.yml   (skip-if-indexed, index-poll, ordered)
           ├─ publish-pypi.yml     (twine check, skip-existing)
           └─ release-cpp-artifacts.yml (build→smoke→pack→attach)
                      └────────────► release.yml (Release + notes + assets)
                                     docs-sites.yml (stable softlink)
```

**Two independent dry-run layers, both available before any immutable artifact:**

1. **Preflight (pre-tag):** everything except the irreversible upload.
2. **Per-publisher dry-run (`workflow_dispatch`):** `cargo publish --dry-run`,
   `use_test_pypi=true`, build-only C++ — each publisher exercisable alone.

**Idempotency everywhere** so a partial/re-triggered release is safe: crates
skip-if-indexed, PyPI `skip-existing`, FFI re-attach via the `version` dispatch
input.

---

## 8. Release runbook (once the above is built)

1. `just release-preflight 2.0.0` — local dry-run green.
2. Run **`release-preflight.yml`** in CI for `2.0.0` — green. (Optionally
   TestPyPI rehearsal, Q6.)
3. `just release-version 2.0.0` — review the diff (version, the `=x.y.z` pins,
   `Cargo.lock`).
4. Open PR "Release 2.0.0"; land through normal CI.
5. `just release-tag && git push --follow-tags`.
6. Tag triggers 6.B–6.E; watch them. Idempotent retries make a re-run safe.
7. Verify: crate pages live; `pip install pyaviso==2.0.0` resolves on each
   target platform; GitHub Release has C++ tarballs + notes; docs `stable`
   points at 2.0.0.
8. Update `progress.md` and `roadmap.md` (move "Release" next → shipped).

---

## 9. Open questions

| # | Question | Default / lean |
|---|----------|----------------|
| Q1 | `finesse` requirement style: `=2.0.0` lockstep vs `^2` float? | Lean float `^2` (it is a generic parser). |
| Q2 | Tag style: bare `2.0.0` (tensogram) vs `v2.0.0`? Workflows can accept both, but pick a canonical for the justfile. | Lean bare `2.0.0`. |
| Q3 | Ship prebuilt C++ libs in 2.0.0 *without* the real-stack C++ e2e gate the roadmap wanted first? | Needs maintainer call. |
| Q4 | Also attach prebuilt `aviso` CLI binaries (Linux/macOS) to the Release? | Optional. |
| Q5 | Changelog: GitHub auto-notes vs a maintained `CHANGELOG.md`? | Lean auto-notes for first cut. |
| Q6 | Full `2.0.0-rc.1` + TestPyPI rehearsal before real 2.0.0? | Recommended for the first ever release. |
| Q7 | FFI packaging: adopt `cargo-c` vs keep hand-rolled `cbindgen` + pack script? | Investigate `cargo-c` fit. |
| Q8 | PyPI auth: OIDC trusted publishing vs API token? | Lean OIDC (no stored secret). |
| Q9 | crates.io index race: sparse-index poll (tensogram) vs retry-on-error (aviso-server)? | Lean sparse-index poll. |
| Q10 | Preflight trigger: manual-only vs also auto on version/manifest PRs? | Lean manual-only first. |

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

## 11. Additional steps to be filled in

> The maintainer has further steps not yet captured here. Add them in this
> section (or inline above) before implementation begins. Known TBD seams:
>
> - (placeholder) …
> - (placeholder) …

---

## 12. Implementation order (once the plan is final — not started yet)

Nothing below is implemented yet; recorded so we can resume after a context
reset.

1. Pre-release cleanups PR (§4): `publish = false` on `aviso-py`, `finesse`
   req style, `cargo-release` config, `justfile`.
2. `release-preflight.yml` (§6.A) — the dry-run gate; validate it on the current
   0.1.0 tree before any bump.
3. `publish-crates.yml` (§6.B) with dry-run path; validate via
   `cargo publish --dry-run`.
4. `publish-pypi.yml` (§6.C) with TestPyPI path; validate via TestPyPI.
5. `release-cpp-artifacts.yml` (§6.D) + `release.yml` (§6.E).
6. Resolve Q1–Q10, do the external prereqs (§10), then the bump → tag → release
   dry-run rehearsal (Q6), then the real 2.0.0.
