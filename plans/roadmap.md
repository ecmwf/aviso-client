# Roadmap

Forward-looking only. What shipped is in [`progress.md`](./progress.md); the
reasoning behind the design is in [`decisions.md`](./decisions.md); the rules
that bound every choice are in [`constraints.md`](./constraints.md).

## Where things stand

aviso-client is unreleased. The Rust core library, the `aviso` CLI, the Python
(PyO3) package, the C++ binding (a stable C ABI plus a header-only C++ facade,
covering the blocking and async verbs, watch, and triggers; D21 steps 1-5), the
end-to-end test suite, self-hosted CI with sccache, and documentation published
to ECMWF Sites are all in `main`. Nothing is published to a package registry
yet.

## What's next

- **Release.** Publish the `aviso` crate to crates.io (Linux only;
  `cargo publish` ships source) and Python wheels to PyPI. The Python package
  will reuse ECMWF's existing `pyaviso` PyPI name, published starting at
  2.0.0 to continue that package's version line. The `aviso` PyPI name itself
  is taken by an unrelated altimetric-data library, which is why the existing
  `pyaviso` name is reclaimed rather than a new one minted. The current
  `pyaviso` project will be archived with a deprecation notice in its README
  and docs pointing here. Wheels need a tag-triggered workflow building
  manylinux (x86_64 and aarch64) plus macOS universal2, `abi3` if supportable,
  and an sdist on every release. No Windows wheels, no musllinux for now (see
  follow-ups). The sdist is the source-build fallback for any platform without
  a matching wheel. macOS regressions surface here rather than in everyday CI.
- **Promote e2e to a merge gate.** The real-stack `e2e` job runs today but stays
  informational. After a flake-free soak, add it to `ci-pass` and require
  `ci-pass` in branch protection.
- **Docs polish.** Realistic end-to-end examples (a MARS consumer, a polygon
  spatial consumer, a multi-listener daemon, a respawn-survival demo), a wider
  troubleshooting section, and CI gates for `clap`-derived reference freshness
  and broken-link / banned-phrase checks.
- **Email trigger.** A fifth trigger kind over `lettre` (SMTP, rustls,
  cross-platform). Operator-configured destinations (Gmail, Outlook / Office365,
  ProtonMail Bridge, a self-hosted Postfix, any RFC-5321 relay); credentials via
  `{{ env.<NAME> }}`. The retry classifier treats SMTP auth failures, malformed
  addresses, and permanent 5xx as terminal. Text-only to start.
- **Prebuilt C++ artifacts.** The C++ binding itself is in `main` (the C ABI,
  the header-only facade, the blocking and async verbs, watch, triggers,
  examples, and docs; D21 steps 1-5). What remains is the packaging deliverable
  that motivated the C-ABI choice: shipping a prebuilt per-platform library so a
  consumer links it with no Rust toolchain. crates.io distributes source only
  (`cargo` compiles it in the consumer's build), so there is no way to fetch a
  compiled `libaviso_ffi.{a,so,dylib}` from it; the libraries must be
  distributed as release assets. Plan: a tag-triggered release workflow that
  cross-builds `libaviso_ffi.{a,so,dylib}` for Linux `x86_64`, Linux `aarch64`,
  and macOS, and attaches each together with `aviso.h` and `aviso.hpp` as GitHub
  Release assets (the drop a CMake consumer points `AVISO_FFI_INCLUDE_DIR` and
  `AVISO_FFI_LIB_DIR` at). The same artifacts are packageable for spack-stack
  and conda-forge with `rust` as a build-only dependency of that recipe.
  Followed by `docs/src/cpp` install/packaging pages. A real-stack C++ e2e CI
  job (the examples run against the e2e stack) lands first, ahead of the release
  matrix. Future work; design context in D21. No Windows.

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
  max-duration cut already covered.
- `StateStoreError` sub-discrimination on the Python side (today it is one
  variant carrying the inner debug string).
- Per-file ECMWF copyright headers if policy requires them (the LICENSE file
  plus per-crate SPDX cover licensing today).
- musllinux wheels (Alpine and other musl distros), added if an Alpine user
  needs them. Manylinux covers every glibc distro; on musl, `pip install`
  falls back to building the sdist from source (needs a Rust and C toolchain),
  so the gap is real but only bites Alpine. Adding it later is a config-only
  change to the wheel matrix.

## Open questions

None gating current work.
