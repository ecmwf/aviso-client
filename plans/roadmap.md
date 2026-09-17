<!--
SPDX-FileCopyrightText: 2026 European Centre for Medium-Range Weather Forecasts (ECMWF)
SPDX-License-Identifier: Apache-2.0
-->

# Roadmap

Forward-looking only. What shipped is in [`progress.md`](./progress.md); the
reasoning behind the design is in [`decisions.md`](./decisions.md); the rules
that bound every choice are in [`constraints.md`](./constraints.md).

## Where things stand

aviso-client 2.0.0 is released: the four crates on crates.io, `pyaviso` on
PyPI (wheels for manylinux x86_64/aarch64 and macOS universal2 plus the
sdist, continuing the legacy line), prebuilt `libaviso_ffi` tarballs on the
GitHub Release, and the `stable` docs link on ECMWF Sites. The release
machinery (preflight gate, invariant-gated publishers, runbook) is in `main`
and documented in CONTRIBUTING.md.

## What's next

- **Docs polish.** Realistic end-to-end examples (a MARS consumer, a polygon
  spatial consumer, a multi-listener daemon, a respawn-survival demo), a wider
  troubleshooting section, and CI gates for `clap`-derived reference freshness
  and broken-link / banned-phrase checks. Plus `docs/src/cpp`
  install/packaging pages for the prebuilt tarballs now on the Release page.
- **Packaging spread.** The release tarballs are packageable for spack-stack
  and conda-forge with `rust` as a build-only dependency of that recipe.

## Follow-ups

Smaller items, none gating. Pick up when the moment is right.

- Server-side `GET /api/v1/auth/check`, then an `aviso auth check` subcommand.
  Dropped from the CLI for now because no such endpoint exists (the schema route
  is anonymous and `/health` is unauthenticated, so neither can validate
  credentials).
- Idempotency-key contract for `POST /api/v1/notification`, so `notify()` can
  safely auto-retry an ambiguous transport failure (today it does not, per D16).
- Strict OpenTelemetry-shaped JSON log formatter (the CLI emits tracing's own
  schema today).
- Server-side replay-summary signal (`requested_cursor`, `oldest_available`,
  `replayed_count`, `complete`); the client ships in completeness-unknown mode
  without it and degrades cleanly.
- An `AVISO_LOG` malformed-directive test once the CLI has runtime tests
  covering it.
- AWS SNS trigger, only if a real user asks (a SigV4-versus-aws-sdk decision
  comes first).
- e2e: optional `pytest-xdist` port-sharding if the suite grows slow, and a
  `docker compose restart` mid-stream reconnect scenario alongside the routine
  max-duration cut already covered. The resume-across-restart test's 5s
  notification deadline has flaked once on a loaded shared host; widen it if
  it recurs.
- `StateStoreError` sub-discrimination on the Python side (today it is one
  variant carrying the inner debug string).
- Per-file ECMWF copyright headers if policy requires them (the LICENSE file
  plus per-crate SPDX cover licensing today).
- musllinux wheels (Alpine and other musl distros), added if an Alpine user
  needs them. Manylinux covers every glibc distro; on musl, `pip install`
  falls back to building the sdist from source (needs a Rust and C toolchain),
  so the gap is real but only bites Alpine. Adding it later is a config-only
  change to the wheel matrix.
- Archive the legacy `ecmwf/aviso` repository now that 2.0.0 has taken over
  the `pyaviso` line (the deprecation notice shipped with 1.0.2). The
  TestPyPI lever was removed from the publish workflow rather than fixing
  the test.pypi.org name ownership; if a sandbox path is ever wanted again,
  `pyaviso` there belongs to an older ECMWF account
  (software.support@ecmwf.int) that would need to add the `ecmwf` user as
  owner.

## Open questions

None gating current work.
