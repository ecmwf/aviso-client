# Architectural decisions

This document is the project's ADR (Architecture Decision Record) log. Every load-bearing decision lives here with the reasoning. New decisions are appended; existing ones are amended in place with a dated note. The decision IDs (D1, D2, …) are stable references used elsewhere in the docs and in commit messages.

The phased roadmap follows the decision list.

---

## D1 — One Rust core, two consumers

A single core library crate (`crates/aviso-client`) implements all behaviour. The Rust CLI (`crates/aviso-client-cli`) and the Python extension (`crates/aviso-client-py`, becoming a `cdylib` in Phase 5) are *peer consumers* and never depend on each other. The core never depends on actix, on PyO3, or on CLI machinery.

Rationale: one code path, two surfaces. Adding a future C/C++ surface becomes a new adapter crate, not a redesign.

---

## D2 — Reconnect-as-norm + at-least-once + checkpointing

`aviso-server` deliberately closes watch connections after `connection_max_duration_sec` (default 3600 s), signalled by a `connection-closing` SSE event with `reason: "max_duration_reached"`. Reconnects are normal operation.

The client:

1. tracks `last_committed_sequence` per resume key;
2. advances it **only after all required triggers for a notification succeed** (at-least-once delivery);
3. reconnects with `from_id = last_committed_sequence + 1`;
4. never checkpoints on heartbeat / control / error / `connection-closing` frames;
5. emits one `INFO` log line `event.name = "client.resume.applied"` on successful resume from stored state.

The watch state is modelled as the orthogonal product of two axes:

```rust
enum ReplayPhase {
    Replaying { start: ResumeStart, replay_completed: bool },
    Live,
    GapDetected { reason: GapReason },
    Closed { reason: CloseReason },
}

enum ConnectionStatus {
    Connected,
    Reconnecting,
    BackoffWait(Duration),
    RefreshingAuth,
}
```

Transitions pass through a single reducer. `Replaying → Live` requires a server-emitted `replay_completed` event. Connection-level events mutate only `ConnectionStatus`. Checkpoint advancement happens only on `commit_notification_after_processing` and never on control frames.

Reconnect classifier:

- `connection-closing.reason = max_duration_reached | end_of_stream` → immediate reconnect, no backoff.
- `connection-closing.reason = server_shutdown` → short backoff (single-digit seconds).
- Transport error → jittered exponential backoff 250 ms → 30 s cap.
- Heartbeat starvation: `max(3 × interval, interval + 30 s)` since the last *any* SSE event (heartbeat, notification, or control). Reset on every event.

Log levels: `max_duration_reached` reconnects at `DEBUG` (routine); first connect / resume-from-stored-state / final give-up at `INFO`; transport errors / heartbeat starvation at `WARN` (rate-limited). Reconnect counts go in a counter, not in chatty logs.

---

## D3 — Resume key

The resume key is a stable hash of:

- normalised server base URL (server identity),
- event type,
- the full canonical watch request body excluding `from_id` and `from_date`,
- schema fingerprint at the time of first use,
- resume-key format version.

Two listeners with different filters get different stored cursors. Two listeners pointing at different servers can never share a cursor. The unhashed normalised fields are stored next to the hash for debugging.

---

## D4 — `StateStore` trait, `MemoryStore` and `JsonFileStore`

- `StateStore` is a small trait (`get`, `set`, `delete` by resume key).
- `MemoryStore` ships in Phase 2 (used by the streaming API).
- `JsonFileStore` ships in Phase 3: JSON under `$XDG_STATE_HOME/aviso-client/state.json`, atomic temp-write + `fsync` + atomic replace, advisory file lock around read-modify-write, monotonic-cursor merge on conflict, network filesystems explicitly unsupported.

SQLite is *not* shipped in v1. It becomes a drop-in `StateStore` impl when users actually need shared durable state across processes or hosts.

---

## D5 — HTTP via `reqwest`, SSE via a parser-only crate

HTTP: `reqwest` with `rustls-tls`. Matches `aviso-server`'s choice.

SSE: a **parser-only** crate (`sse-core`, with `eventsource-stream` as fallback). The reconnect loop is *ours*, not the SSE crate's. The reason is structural: high-level SSE client crates (`eventsource-client`, `reqwest-eventsource`) drive reconnects using the WHATWG `Last-Event-ID` mechanism, which `aviso-server` does not honour. The server's resume contract is `from_id` / `from_date` in the POST body, which requires a re-POST on every reconnect — incompatible with the high-level crates' assumptions.

---

## D6 — Runtime: tokio

`tokio` matches `aviso-server`, matches the Rust async ecosystem we'll touch, and is what `pyo3-async-runtimes` is built against.

---

## D7 — No client-side schema validation

The server is the single source of truth for validation. The client does not depend on `aviso-validators` and does not perform pre-flight validation. The `GET /api/v1/schema` endpoint is still exposed via the CLI for human discovery (`aviso-client schema list`/`get`), but no validation pipeline runs on the client side.

On a validation failure from the server, the client surfaces the server's error verbatim, with the `X-Request-ID` for correlation.

Rationale: eliminates an entire class of drift bugs; removes a dependency and a version-pin headache; respects the user-stated principle that validation belongs on one side only and the server already returns good errors.

---

## D8 — `AuthProvider` trait + config sources

`AuthProvider` is an **async** trait. v1 implementations: `BasicAuth`, `BearerToken`, `EnvAuth`, `ConfigFileAuth`, `Chain(...)`. Sources are composable, precedence is `explicit > env > file > defaults`.

Environment variables: `AVISO_USERNAME`, `AVISO_PASSWORD`, `AVISO_TOKEN`, `AVISO_BASE_URL`, `AVISO_CLIENT_CONFIG_FILE`. The `AVISO_*` prefix matches the legacy `pyaviso` convention where semantics overlap; `AVISO_CLIENT_*` is reserved for nested config overrides specific to the new client.

On a 401, the provider may refresh credentials and the request is retried once if it is safe to retry. Streaming reconnects use refreshed credentials. Token contents are never logged.

---

## D9 — CloudEvent envelope hidden

Users see `Notification { sequence: u64, topic, event_id, event_type, time, payload: serde_json::Value, metadata }`. The CloudEvent envelope is parsed internally. Sequence is extracted from the CloudEvent `id` field of the form `<event_type>@<sequence>`, with `rsplit_once('@')` so an event type containing `@` does not break extraction. A `id` that fails to parse is surfaced as a `client.sse.event.malformed` ERROR and triggers a reconnect.

---

## D10 — C++ surface deferred; Rust API is for Rust users

A C/C++ surface is not implemented in v1. The public Rust API is **not** constrained for hypothetical FFI compatibility — closures, generics, and traits are fair game. Data models stay FFI-friendly where cheap. When the C++ surface is eventually built, it lives in a *separate adapter crate* (hand-written C ABI + `cbindgen` as a header mirror, or `cxx` for a richer C++ bridge) and translates from the Rust API.

---

## D11 — Triggers

v1 ships two trigger kinds only: `echo` (stdout) and `log` (file). Both are implemented in Rust core (`crates/aviso-client/src/triggers/`). The CLI YAML loader and the Python wrapper both consume the same dispatcher.

The dispatcher is **internal in v1**: an `enum Trigger { Echo(...), Log(...) }` plus a dispatcher function. There is no public `Trigger` trait until a third trigger kind appears. Required vs optional triggers, per-trigger timeout, bounded retry+backoff, and per-trigger `fail_fast` flag are part of the framework from day one.

Checkpoint policy: a notification's `last_committed_sequence` advances **only** after all required triggers succeed. Optional triggers may fail without blocking the checkpoint, but must be marked optional explicitly. If required triggers exhaust their retries, the listener stops and the checkpoint stays where it was.

Naming aligns with legacy `pyaviso` (`echo`, `log`) so existing operator habits transfer.

---

## D12 — Logging per ECMWF Codex `Observability.md`

- Libraries (`aviso-client`, `aviso-client-py`) never configure global logging. They emit `tracing` events with stable `event.name` strings: `client.sse.*`, `client.resume.*`, `client.schema.*`, `client.auth.*`, `client.trigger.*`, `client.notify.*`.
- The CLI binary owns subscriber init: JSON to stderr by default, `AVISO_LOG` (full `EnvFilter` syntax) overrides level/target.
- The Python extension bridges Rust `tracing` to Python `logging` via `pyo3-log` with `Caching::LoggersAndLevels`, initialised once at module init.
- Single-boundary log discipline: lower layers *return* structured errors; the reconnect supervisor logs classification and final outcome. On give-up, one primary `ERROR` carries the full error chain.
- Redaction at emission for known sensitive headers (`Authorization`, `Cookie`) and field names matching `password|token|secret|api_key`. URLs are sanitised (userinfo + sensitive query params removed). Request/response bodies are not logged by default; notification payloads are treated as sensitive unless explicitly enabled.
- Every log line carries `request_id` when known and `resume_key` in a watch context.

---

## D13 — License Apache-2.0 + ECMWF header

Same regime as `aviso-server`.

---

## D14 — Metrics deferred to v1.1

No `prometheus`/`metrics` dependency ships in v1. The reserved namespace and label policy are documented here so the v1.1 implementation is mechanical:

- `aviso_client_reconnects_total{reason,outcome}`
- `aviso_client_notifications_processed_total{outcome}`
- `aviso_client_trigger_duration_seconds{trigger_kind,outcome}`

Banned labels (ECMWF Codex high-cardinality rule): `request_id`, `resume_key`, raw URL, username, UUID, payload fields, full event identifiers (`<event_type>@<sequence>`).

---

## D15 — Watch state machine: orthogonal product

See D2. `ReplayPhase × ConnectionStatus`, single reducer, fields private. Tests must exercise invalid transition attempts through the public API and not by mutating fields.

---

## D16 — `notify()` is not auto-retried on ambiguous transport failure

A `POST /api/v1/notification` that fails with an ambiguous transport error after the request body has been sent may have been processed by the server. The client does not retry it. Failures before request-body transmission may be retried where `reqwest` can classify them safely.

A server-side idempotency-key contract would lift this restriction; it is an open ask, not a current dependency.

---

## D17 — `from_date` is bootstrap-only

A user-supplied `from_date` is used only for the very first connection of a listener. After the first notification is committed, the cursor switches to sequence-based (`from_id = last_committed_sequence + 1`). Server time is authoritative; the client does not attempt clock-skew compensation.

---

# Phased roadmap

Each phase has explicit scope, out-of-scope, "done" definition, and documentation expectations. "Done" requires CI green on every check, not just the new feature.

## Phase 0 — Bootstrap *(this PR)*

- Workspace skeleton (three crates, all building).
- mdBook skeleton with `SUMMARY.md` mirroring the planned taxonomy.
- `pyproject.toml` (maturin backend, scaffold only).
- CI: `cargo fmt --check`, `cargo clippy -D warnings`, `cargo test --workspace`, `mdbook build docs`, `cargo deny check`.
- `tests/e2e/` docker-compose pinned to a specific `aviso-server` commit SHA.
- First ADR set (this document).

**Out of scope**: any client behaviour.

**Done when**: a fresh clone passes the five CI commands above.

## Phase 1 — Rust core (non-streaming)

- `AvisoClient`: `notify`, `schema`, `schema/{event_type}` pass-through, admin endpoints.
- `AuthProvider` (async) + `BasicAuth`, `BearerToken`, `EnvAuth`, `ConfigFileAuth`, `Chain`.
- `ClientError` (thiserror enum with `X-Request-ID` correlation surfaced).
- `Notification` / `NotificationRequest` types; CloudEvent sequence extraction.
- Conservative `notify()` retry policy (D16).
- Server-version handshake hooks: gracefully degrade if the server does not report it.

**Tested with**: `wiremock`. Property tests on CloudEvent id parsing.

**Docs**: usage/rust-quickstart, configuration/auth.

## Phase 2 — Streaming + checkpoint semantics

- `sse-core` parser behind our own `SseEvent` enum.
- Reconnect supervisor with classifier; single-boundary log discipline.
- Watch state machine per D2/D15.
- `StateStore` trait + `MemoryStore`.
- `HistoryGap` typed error (D2).
- Trigger dispatcher + `Echo` + `Log` (D11), wired into checkpoint policy.
- Reserved metric names doc-only (D14).
- **8-test PyO3 spike** to validate `__aiter__`/`__anext__` + cancellation + `aclose()` + dropped-iterator-no-leak + bounded-queue-backpressure + typed-exception + post-`aclose` `StopAsyncIteration` + multi-iterator shared runtime.

**Tested with**: `wiremock` chunked-response harness, kill-every-200ms stress test, heartbeat-starvation, drop-during-replay, schema-fingerprint-change, replay-limit-mid-stream.

**Docs**: resume/sse-and-reconnects, triggers/echo, triggers/log.

**Public streaming API is *not* declared stable here.** That happens at the end of Phase 3.

## Phase 3 — Resume persistence

- `JsonFileStore` per D4.
- Kill-and-restart + concurrent-process tests.
- `client.resume.applied` INFO log on resume from stored state.

**Done when**: the streaming API is stable; Phase 2 + Phase 3 tests all pass.

## Phase 4 — Rust CLI

- Subcommands: `notify`, `watch`, `replay`, `schema {list,get}`, `admin {wipe-stream,wipe-all,delete}`, `auth check`, `config dump --redact`.
- YAML config + `AVISO_CLIENT_CONFIG_FILE` override + `AVISO_CLIENT_*` nested env vars.
- `--yes` required for destructive admin commands.
- `--ca-bundle <path>` for dev TLS; `--danger-accept-invalid-certs` loudly named with a WARN log.
- Graceful shutdown via cancellation tokens.

**Tested with**: `assert_cmd`.

## Phase 5 — PyO3 extension

- `AvisoClient` (async).
- `AsyncWatchIter`: manually implemented `__aiter__`/`__anext__` over `tokio::sync::mpsc`; owns a `CancellationToken`.
- `BlockingAvisoClient`: refuses if `asyncio.get_running_loop()` returns; context-manager required; `KeyboardInterrupt` cancels the Rust task.
- Single tokio runtime shared per Python process.
- `pyo3-log` bridge with `Caching::LoggersAndLevels`.
- `pyo3-stub-gen` type stubs.

**Ends with**: a single Linux wheel built in CI + import-smoke test.

## Phase 6 — Wheel matrix

- `cibuildwheel`: Linux x86_64/aarch64 (manylinux + musllinux), macOS universal2, Windows x86_64; `abi3` if possible.

## Phase 7 — Docs polish + examples

- Realistic end-to-end examples (MARS consumer, polygon spatial consumer, multi-listener daemon, respawn-survival demo).
- Troubleshooting expansion.
- `mdbook test` of code blocks in CI.

## Phase 8 (deferred) — C++ FFI surface

Only on user request. The Rust API is not constrained for it (D10); a future adapter crate translates.
