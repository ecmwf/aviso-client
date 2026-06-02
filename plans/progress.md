# Progress

What has shipped, newest first. One line per merged PR; open the PR or read
[`decisions.md`](./decisions.md) for the detail.

## Capabilities today

A Rust core library, an `aviso` CLI, and a Python package, all over one
implementation: publish notifications, watch and replay streams with automatic
reconnect and at-least-once resume, and run echo / log / command / webhook
triggers from YAML. Backed by a real-stack end-to-end suite, self-hosted CI with
sccache, and documentation published to ECMWF Sites.

## Log

- [#34](https://github.com/ecmwf/aviso-client/pull/34) ffi: the rest of the
  blocking C++ surface. Adds `notify`, `schema_for`, `wipe_stream`, `wipe_all`,
  and `delete_notification` to the C ABI and the C++ facade, derives `Serialize`
  on `NotifyResponse`, and ships a `publish` example plus Publishing and
  Operations docs. Verified end to end against the e2e stack.
- [#33](https://github.com/ecmwf/aviso-client/pull/33) ffi: C++ binding
  foundation. A peer crate `crates/aviso-ffi` exposes the client as a stable C
  ABI (cbindgen-generated `aviso.h`, owning handles, a structured-error
  outcome, a process-global runtime, panic-guarded entry points) plus a
  header-only C++ facade, with client construction and the blocking schema verb.
  A CMake example and a self-hosted, sccache-backed CI job build and run it.
- [#26](https://github.com/ecmwf/aviso-client/pull/26) docs: README aviso logo,
  badges, and hosted-docs links.
- [#25](https://github.com/ecmwf/aviso-client/pull/25) ci: publish the mdBook to
  ECMWF Sites (canonical site plus PR previews).
- [#24](https://github.com/ecmwf/aviso-client/pull/24) ci: self-hosted runners
  with S3-backed sccache; real-stack e2e job (informational).
- [#23](https://github.com/ecmwf/aviso-client/pull/23) py: examples lead with
  the local docker-compose stack.
- [#22](https://github.com/ecmwf/aviso-client/pull/22) test: end-to-end suite
  against a real aviso-server + auth-o-tron + NATS stack.
- [#21](https://github.com/ecmwf/aviso-client/pull/21) py: `triggers=` kwarg,
  examples, and iterator-as-context-manager.
- [#20](https://github.com/ecmwf/aviso-client/pull/20) py: Python PyO3 API
  (async and blocking clients, watch iterator, auth, state stores, triggers).
- [#16](https://github.com/ecmwf/aviso-client/pull/16) cli: `aviso` binary
  (notify, listen, replay, schema, admin, config, completions).
- [#15](https://github.com/ecmwf/aviso-client/pull/15) core: webhook trigger and
  YAML `TriggerConfig` deserialiser.
- [#14](https://github.com/ecmwf/aviso-client/pull/14) core: command trigger
  with bounded I/O, template engine, and timeout / fail-fast.
- [#13](https://github.com/ecmwf/aviso-client/pull/13) refactor: decompose
  oversized modules into sub-module trees.
- [#12](https://github.com/ecmwf/aviso-client/pull/12) core: cross-process lock
  and monotonic merge for `JsonFileStore`.
- [#11](https://github.com/ecmwf/aviso-client/pull/11) core: amend D14 to
  tracing-only; drop the dead `FatalKind` variant.
- [#10](https://github.com/ecmwf/aviso-client/pull/10) core: trigger dispatch
  (echo and log) with required / optional semantics.
- [#9](https://github.com/ecmwf/aviso-client/pull/9) core: watch resilience
  (reconnect, auth refresh, heartbeat watchdog, state-store resume).
- [#8](https://github.com/ecmwf/aviso-client/pull/8) core: watch API and
  single-connection supervisor.
- [#7](https://github.com/ecmwf/aviso-client/pull/7) core: pure-logic watch
  state-machine reducer.
- [#6](https://github.com/ecmwf/aviso-client/pull/6) core: `StateStore` trait
  with `MemoryStore` and single-process `JsonFileStore`.
- [#5](https://github.com/ecmwf/aviso-client/pull/5) core: `finesse` WHATWG SSE
  parser crate.
- [#3](https://github.com/ecmwf/aviso-client/pull/3) core: Rust core (notify,
  schema, admin; non-streaming).
- [#2](https://github.com/ecmwf/aviso-client/pull/2) core: error type,
  notification model, and auth providers.
- [#1](https://github.com/ecmwf/aviso-client/pull/1) chore: bootstrap workspace,
  mdBook skeleton, CI gates, and the e2e compose file.

[#4](https://github.com/ecmwf/aviso-client/pull/4) was a plans-only commit
recording early streaming decisions.
