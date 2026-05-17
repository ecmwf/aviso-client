# TODO

> Optional, short, active-phase notes only. Durable planning lives in
> GitHub Issues + milestones. The phased roadmap lives in
> [`docs/src/internals/decisions.md`](docs/src/internals/decisions.md).

## Active phase: Phase 1 — Rust core (non-streaming)

See [`docs/src/internals/decisions.md`](docs/src/internals/decisions.md#phase-1--rust-core-non-streaming) for the scope and acceptance criteria.

## Carried forward from Phase 0

- mdBook link checker (`mdbook-linkcheck` or equivalent) — anchor-link bug found by hand in Pass 5; add a guard so the next one is caught automatically.
- Windows in the CI matrix — added in Phase 6 when wheels need it, unless Phase 1 surfaces platform-specific behaviour earlier.
- ADR D12 OTel-alignment is *aspirational*: the current CLI emits `tracing_subscriber::fmt::json()` output which uses tracing's own field schema, not OTel's. A custom formatter is a Phase 1 follow-up if strict OTel-alignment is needed.
- ECMWF per-file copyright header (ADR D13 was softened to license-only); revisit if ECMWF requires the boilerplate.

## Open questions

None gating Phase 1. Server-side validator-version and replay-completeness signals remain unrequired; the client ships in completeness-unknown mode (see ADR D7, D15).
