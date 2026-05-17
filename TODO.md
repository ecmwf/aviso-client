# TODO

> Optional, short, active-phase notes only. Durable planning lives in
> GitHub Issues + milestones. The phased roadmap lives in
> [`docs/src/internals/decisions.md`](docs/src/internals/decisions.md).

## Active phase: Phase 0 — Bootstrap

- [x] Workspace skeleton (three crates, all building)
- [x] mdBook skeleton with `SUMMARY.md` mirroring the planned taxonomy
- [x] `pyproject.toml` (maturin backend, scaffold only)
- [x] LICENSE / README / CONTRIBUTING / TODO
- [x] CI: `cargo check`, `cargo fmt --check`, `cargo clippy -D warnings`, `cargo test --workspace`, `mdbook build`, `cargo deny check`
- [x] `tests/e2e/` docker-compose pinned to aviso-server commit SHA
- [x] First ADR set committed (decisions D1–D17 from plan v0.3)

## Up next: Phase 1 — Rust core (non-streaming)

See [`docs/src/internals/decisions.md`](docs/src/internals/decisions.md#phase-1).

## Open questions

None gating Phase 0. Server-side validator-version + replay-completeness signals are *not* required; the client ships in completeness-unknown mode by default (see ADR D7, D15).
