# Architectural decisions

This document is the project's ADR (Architecture Decision Record) log. Durable
architectural decisions live here with their reasoning; finer implementation
choices live in the linked PRs and the code. New decisions are appended;
existing ones are amended in place with a dated note. The decision IDs (D1, D2,
and so on) are stable references used elsewhere in the docs and in commit
messages.

What is next is in [`roadmap.md`](./roadmap.md); what has shipped is in
[`progress.md`](./progress.md). Both reference ADRs by id. The server facts that
drive these decisions are collected at the end of this file.

---

## D1. One Rust core, two consumers

A single core library crate (`crates/aviso`, published as `aviso`) implements
all behaviour. The Rust CLI (`crates/aviso-cli`, producing the `aviso` binary)
and the Python extension (`crates/aviso-py`, the PyO3 binding crate; becomes a
`cdylib` once bindings are added) are *peer consumers* and never depend on each
other. The core never depends on actix, on PyO3, or on CLI machinery.

The repository name (`aviso-client`) intentionally differs from the crate names:
the repo is the *project*, while the crates inside it live in the unprefixed
`aviso` namespace because their consumers (Cargo, `cargo install`,
`pip install`, `import`) read the crate/package name, not the repo path.

Rationale: one code path, two surfaces. Adding a future C/C++ surface becomes a
new adapter crate, not a redesign.

*Amendment, 2026-06-10*: the Python distribution and importable module are named
`pyaviso`, not `aviso`. The unprefixed `aviso` name is taken on PyPI by an
unrelated project, so the package reuses ECMWF's existing `pyaviso` name (see
`roadmap.md`). The Rust crates (`aviso`, `aviso-cli`, `aviso-py`) and the
installed CLI binary (`aviso`) keep their names. The `pyaviso` wheel also
bundles the `aviso` command-line tool as a console script, so a single
`pip install pyaviso` provides both `import pyaviso` and the `aviso` command,
matching the single-wheel ergonomics of tools like `uv`. maturin cannot ship a
PyO3 extension and a separate `bin` artefact in one wheel, so the bundling works
by exposing the CLI through the extension: `aviso-cli` grows a library target
whose `run` entry point both the `aviso` binary and the extension call, the
`aviso-py` extension depends on that library target and exposes a private
`_run_cli`, and a PEP 621 `[project.scripts]` entry bridges the `aviso` console
command to it. This narrows the original "the CLI and the Python extension never
depend on each other" clause: `aviso-py` may depend on `aviso-cli` solely to
bundle the console command. The core crate (`aviso`) stays free of PyO3 and CLI
machinery, and the CLI stays independent of the extension.

---

## D2. Reconnect-as-norm, at-least-once, checkpointing

`aviso-server` deliberately closes watch connections after
`connection_max_duration_sec` (default 3600 s), signalled by a
`connection-closing` SSE event with `reason: "max_duration_reached"`. Reconnects
are normal operation.

The client:

1. tracks `last_committed_sequence` per resume key;
2. advances it **only after all required triggers for a notification succeed**
   (at-least-once delivery);
3. reconnects with `from_id = last_committed_sequence + 1`;
4. never checkpoints on heartbeat, control, error, or `connection-closing`
   frames;
5. emits one `INFO` log line `event.name = "client.resume.applied"` on
   successful resume from stored state.

The watch state is modelled as the orthogonal product of two axes:

```rust,ignore
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

Transitions pass through a single reducer. The transition from `Replaying` to
`Live` requires a server-emitted `replay_completed` event. Connection-level
events mutate only `ConnectionStatus`. Checkpoint advancement happens only on
`commit_notification_after_processing` and never on control frames.

Reconnect classifier. Close-reason behaviour depends on the active operation:

- `connection-closing.reason = max_duration_reached`: immediate reconnect, no
  backoff. Server-policy close; always routine.
- `connection-closing.reason = server_shutdown`: short backoff (single-digit
  seconds), then reconnect.
- `connection-closing.reason = end_of_stream`:
  - In **watch** (live or historical-then-live): treat as graceful close,
    immediate reconnect with the next sequence.
  - In **replay-only** AFTER a `replay_completed` control event: terminal; do
    not reconnect. The replay has finished naturally.
  - In **replay-only** BEFORE `replay_completed`: connection dropped mid-replay;
    reconnect from `last_committed_sequence + 1` (or original
    `from_id`/`from_date` if no notification has been committed yet).
- Transport error: jittered exponential backoff 250 ms to 30 s cap.
- Heartbeat starvation: `max(3 × interval, interval + 30 s)` since the last SSE
  event of any kind (heartbeat, notification, or control). Reset on every event.

Log levels: `max_duration_reached` reconnects at `DEBUG` (routine); first
connect, resume-from-stored-state, and final give-up at `INFO`; transport errors
and heartbeat starvation at `WARN` (rate-limited). Reconnect counts go in a
counter, not in chatty logs.

---

## D3. Resume key

The resume key is a stable hash of:

- normalised server base URL (server identity),
- event type,
- the full canonical watch request body excluding `from_id` and `from_date`,
- schema fingerprint at the time of first use,
- resume-key format version.

Two listeners with different filters get different stored cursors. Two listeners
pointing at different servers can never share a cursor. The hash is the only key
stored on disk; unhashed components are not persisted because the trade-off
(file size grows with every subscription, debugging value is marginal once
`tracing` logs already include the resume key hex) does not pay off.

*Amendment, 2026-05-18*: the original text said "the unhashed normalised fields
are stored next to the hash for debugging". They are not. The file format stores
hex digest keys only. Operators recover the unhashed components from the same
`tracing` events that emit the key in the first place.

---

## D4. `StateStore` trait, `MemoryStore` and `JsonFileStore`

- `StateStore` is a small async trait (`get`, `put`, `delete` by resume key).
- `MemoryStore` is the in-process implementation, appropriate for tests,
  short-lived consumers, and any path that does not need to survive process
  restarts.
- `JsonFileStore` provides on-disk persistence safe for cooperating processes on
  local filesystems:
  - **Atomic write**: a JSON file with atomic temp-write plus `fsync` of the
    file plus atomic rename plus `fsync` of the parent directory (Windows uses
    `MoveFileExW` with `MOVEFILE_WRITE_THROUGH | MOVEFILE_REPLACE_EXISTING`). A
    `kill -9` mid-write cannot corrupt the existing file.
  - **Cross-process advisory lock**: an exclusive `flock` (Unix via `rustix`,
    Windows via `LockFileEx`) on a sidecar lockfile at `<state_path>.lock`
    serialises writes between processes. The sidecar is chosen over the data
    file itself because the atomic-rename pattern replaces the data file's inode
    and would invalidate the lock; the sidecar is never renamed, so its lock
    semantics stay stable.
  - **Monotonic-cursor merge**: the store re-reads disk under the lock and
    merges against the in-memory candidate before each write. A `put` whose
    `last_committed_sequence` is less than or equal to the durable value is
    silently a no-op (callers needing to reset must `delete` first, then `put`).
    A `delete` is suppressed when a concurrent writer has advanced the key past
    the value the caller observed at delete time. These rules together preserve
    at-least-once delivery across both single-process and cross-process races.
  - **Concurrency contract**: a successful `put` is durable-before-visible; a
    failed `put` leaves both in-memory and on-disk state unchanged. Writes from
    cooperating processes are serialised through the cross-process lock, and the
    strict-monotonic merge guarantees no committed checkpoint moves backwards.
    Reads (`get`) return this handle's in-memory snapshot, which can be stale
    relative to a sibling process's writes between this handle's own
    write-through cycles; it is consistent within a `Clone` group of the same
    `JsonFileStore` handle.
  - **Local filesystems only**: `flock` is not safe over NFS or CIFS; documented
    as a precondition.
  - **Lockfile lifetime**: the lockfile is created lazily on first `put` and
    never deleted by the library; operators must not delete or replace it either
    (deleting and recreating the path lets two writers acquire "the" lock on two
    different inodes).
  - **CLI default**: `~/.config/aviso/state.json` once the CLI wires
    `StateStore`; the wiring is a follow-up.

The `StateStore` trait is the extension point and is itself strictly monotonic
in `put` so the `MemoryStore` and `JsonFileStore` are observationally identical.
Future backends can plug in by implementing the trait; none are planned in this
codebase.

*Amendment, 2026-05-18*: the original scope bundled the multi-process
correctness work with the file store from day one. Pulled the single-process
implementation forward so the CLI binary can resume across restarts without
waiting for the cross-process work to settle. The multi-process correctness
story above is unchanged in substance, only deferred.

*Amendment, 2026-05-18*: the second of the three trait methods was renamed from
`set` to `put`. `put` matches REST-idiomatic key-value semantics ("put a value
at a key") and aligns with the persistent-store crates the codebase is likeliest
to grow into (`sled`, `rocksdb`). Existing call sites do not yet exist outside
the new state module, so the rename is mechanical.

*Amendment, 2026-05-19*: the deferred multi-process correctness layer landed.
The chosen advisory-lock crate is `fd-lock` v4.x, picked over `fs2` because
fd-lock is actively maintained (last commit 2025-04-23, three owners), uses
`rustix` on Unix and `windows-sys` on Windows, and exposes an `RwLock<T>`
wrapper with RAII guards. The lock subject is a sidecar lockfile at
`<state_path>.lock` rather than the data file itself because the atomic-rename
pattern replaces the data file's inode and would invalidate the lock. The merge
step is strict monotonic: `put` with a sequence less than or equal to the
existing on-disk value is a silent no-op (`delete` first, then `put`, is the
explicit reset workflow). `MemoryStore` is updated to match the strict-monotonic
contract so both implementations are observationally identical. The shipped
`JsonFileStore` and `MemoryStore` are validated by a multi-process
kill-during-write fuzz harness that spawns four child processes per wave, kills
them all after 500 ms with `SIGKILL` (Unix) or `TerminateProcess` (Windows), and
asserts the surviving file parses cleanly and no key's stored sequence has
decreased.

---

## D5. HTTP via `reqwest`, SSE via the owned `finesse` parser

HTTP: `reqwest` with `rustls-tls`. Matches `aviso-server`'s choice.

SSE: an owned parser-only workspace crate,
[`crates/finesse`](https://github.com/ecmwf/aviso-client/tree/main/crates/finesse)
(named for the SSE that hides in the middle of the word). It implements the
WHATWG parsing algorithm from the
[HTML Living Standard](https://html.spec.whatwg.org/multipage/server-sent-events.html)
sections 9.2.5 and 9.2.6 as a sync push-based API: bytes go in via `feed`,
frames come out via `next_frame`, end-of-stream is signalled via `end`. The
reconnect loop is *ours*, not the parser's; the parser owns no transport, no
async runtime, no aviso semantics.

The reason for owning the parser is structural. High-level SSE client crates
(`eventsource-client`, `reqwest-eventsource`) drive reconnects using the WHATWG
`Last-Event-ID` mechanism, which `aviso-server` does not honour. The server's
resume contract is `from_id` or `from_date` in the POST body, which requires a
re-POST on every reconnect. That is incompatible with the high-level crates'
assumptions. Parser-only crates on crates.io were either too fresh to depend on,
maintenance-cold for years, or shipped reconnect logic glued to their parser.
`sse-codec` (MPL-2.0, mature, parser-only) was the closest match, but keeping
the parser in-tree puts spec-conformance fixes and any future aviso-driven
extensions on our critical path rather than upstream's.

The parser is `publish = false` until a second consumer needs it (likely the
`aviso-py` extension). The first external consumer revisits that.

*Amendment, 2026-05-18*: this ADR originally named `sse-core` as the chosen
third-party parser-only crate. The decision was revised before any client code
shipped against it. The current text reflects the owned-parser approach.

---

## D6. Runtime: tokio

`tokio` matches `aviso-server`, matches the Rust async ecosystem we will touch,
and is what `pyo3-async-runtimes` is built against.

---

## D7. No client-side schema validation

The server is the single source of truth for validation. The client does not
depend on `aviso-validators` and does not perform pre-flight validation. The
`GET /api/v1/schema` endpoint is still exposed via the CLI for human discovery
(`aviso schema list`/`get`), but no validation pipeline runs on the client side.

On a validation failure from the server, the client surfaces the server's error
verbatim, with the `X-Request-ID` for correlation.

Rationale: eliminates an entire class of drift bugs; removes a dependency and a
version-pin headache; respects the user-stated principle that validation belongs
on one side only and the server already returns good errors.

---

## D8. `AuthProvider` trait and config sources

`AuthProvider` is an **async** trait. v1 implementations: `BasicAuth`,
`BearerToken`, `EnvAuth`, `ConfigFileAuth`, `Chain(...)`. Sources are
composable, precedence is `explicit > env > file > defaults`.

Environment variables: `AVISO_USERNAME`, `AVISO_PASSWORD`, `AVISO_TOKEN`,
`AVISO_BASE_URL`, `AVISO_CLIENT_CONFIG_FILE`. The `AVISO_*` prefix matches the
legacy `pyaviso` convention where semantics overlap; `AVISO_CLIENT_*` is
reserved for nested config overrides specific to the new client.

On a 401, the provider may refresh credentials and the request is retried once
if it is safe to retry. Streaming reconnects use refreshed credentials. Token
contents are never logged.

---

## D9. CloudEvent envelope hidden

Users see
`Notification { event_type, sequence: u64, identifier, payload: serde_json::Value, request_id }`.
The CloudEvent envelope is parsed internally. Sequence is extracted from the
CloudEvent `id` field of the form `<event_type>@<sequence>`, with
`rsplit_once('@')` so an event type containing `@` does not break extraction.

The struct is `#[non_exhaustive]` because envelope-derived fields land
incrementally as the streaming surface ships: `event_id` (the raw
`<event_type>@<sequence>` string, preserved for support correlation), `time`
(CloudEvent emission time), `source` (CloudEvent source URI), and any extension
attributes as `metadata` will be added when the SSE mapper lands. The
non-streaming surface only needs `event_type`, `sequence`, `identifier`, and
`payload`; `request_id` is the per-response HTTP correlation surface that exists
everywhere outside the SSE stream.

A malformed `id` (no `@`, non-numeric suffix, or `u64` overflow) is a **terminal
protocol error**, not a reconnect trigger. If the server is emitting malformed
ids deterministically, reconnecting would re-receive the same bad event and the
client would livelock. The client logs one `ERROR` with
`event.name = "client.sse.event.malformed"`, the raw id string (sanitised), the
`request_id`, the topic, and the current resume key, then closes the stream with
a typed `ClientError::MalformedEvent` for the user to decide what to do
(typically: file a bug, optionally restart the watch with a fresh cursor).

---

## D10. C++ surface deferred; Rust API is for Rust users

A C/C++ surface is not implemented in v1. The public Rust API is **not**
constrained for hypothetical FFI compatibility: closures, generics, and traits
are fair game. Data models stay FFI-friendly where cheap. When the C++ surface
is eventually built, it lives in a *separate adapter crate* (hand-written C ABI
plus `cbindgen` as a header mirror, or `cxx` for a richer C++ bridge) and
translates from the Rust API.

---

## D11. Triggers

The core ships four trigger kinds: `echo` (NDJSON to stdout), `log` (NDJSON to a
file), `command` (subprocess spawn, Unix-only), and `webhook` (HTTP request,
cross-platform). All four are implemented in Rust core under
`crates/aviso/src/watch/trigger/` as a sibling module to the other watch
internals. The CLI YAML loader and the Python wrapper consume the same
dispatcher.

The dispatcher is **internal**: a crate-private
`enum TriggerKind { Echo, Log { path }, Command(Box<CommandConfig>), Webhook(Box<WebhookConfig>) }`
plus a `dispatch_triggers` function inside the trigger module. There is no
public `Trigger` trait; the public surface is the builder-style `Trigger` struct
(with `echo()`, `log(path)`, `command(cmd)`, and `webhook(url)` constructors
plus `.retries(u32)`, `.required(bool)`, `.timeout(Duration)`,
`.fail_fast(bool)`, `.env(k, v)`, `.working_dir(p)`, `.method(HttpMethod)`,
`.header(name, value)`, and `.body_template(body)` setters), the public
`TriggerKindLabel` and `TriggerError` types that appear on
`ClientError::TriggerFailed`, the public `TemplateErrorKind` type carried by
`TriggerError::Template`, the public `HttpMethod` enum (Post / Get / Put / Patch
/ Delete), the public `TriggerConfig` enum and its payload structs
(`EchoConfig`, `LogConfig`, `CommandTriggerConfig`, `WebhookTriggerConfig`) for
declarative YAML configuration, and `WatchRequest::with_triggers(Vec<Trigger>)`.

Per-trigger tunables: `retries: u32` (default `0`) with the supervisor's
standard exponential backoff schedule, `required: bool` (default `true`),
`timeout: Option<Duration>` (default `None` for echo / log / command,
`DEFAULT_WEBHOOK_TIMEOUT` (30 s) for webhook; meaningful for command and
webhook, silently ignored on echo / log), and `fail_fast: bool` (default `true`;
meaningful for command and webhook, silently ignored on echo / log). The command
trigger additionally accepts an env-var map via `.env(k, v)` and an override
working directory via `.working_dir(p)`. The webhook trigger additionally
accepts a method via `.method(HttpMethod)`, repeated headers via
`.header(name, value)`, and a body override via `.body_template(body)`. All
setters live on the parent `Trigger` so the builder API stays uniform; setters
that have no effect on a particular kind document that explicitly in their
rustdoc.

The command trigger renders its command string through a small in-crate template
engine that recognises `{{ notification.<dotted.path> }}` and `{{ env.<NAME> }}`
expressions. The trigger constructors are infallible (matching the existing
`echo()` / `log(path)` shape); a malformed template surfaces at first dispatch
as `TriggerError::Template { context, field, kind }` where `context` is a safe
static label (`"command"`) set at the dispatch boundary and the raw template
never reaches the public error chain. Scalar string values render unquoted;
numbers, booleans, and nulls render via Display; objects and arrays render as
compact JSON. There is no `NotString` template error: every JSON value type
produces a substituted string under the resolution rules.

Command-trigger output capture uses a 4 KiB ring buffer per stream (stdout and
stderr). Concurrent draining tasks prevent the kernel pipe buffer from filling
(Linux's default 64 KiB) and blocking the child. Stdout content is suppressed
per the no-payload-logging discipline (only byte count appears in DEBUG-level
tracing); stderr tail goes into the public
`TriggerError::Command { exit_code, stderr_tail }` variant on non-zero exit. The
dispatcher honours `Trigger::timeout` by racing `child.wait()` against
`tokio::time::sleep(t)` in a `tokio::select!`; on timeout it issues `SIGKILL`
(logging any error), reaps the zombie (logging any error), aborts the drain
tasks, and returns `TriggerError::Timeout(t)`. Shell descendant cleanup is a
documented limitation: `child.kill()` reaches only the `/bin/sh -c ...` child,
not pipelines or backgrounded jobs; operators who need full process-tree cleanup
use `exec ./binary` so the shell PID equals the target binary's PID. A 5-second
post-exit drain cap bounds the dispatcher's exposure even when descendants leak.
Command-trigger support is POSIX-only: the `Trigger::command` API and the
related `TriggerKindLabel::Command` / `TriggerError::Command` variants are gated
behind `#[cfg(unix)]`. Windows builds of the crate compile cleanly without them
and the echo / log triggers continue to work; native Windows support for the
command trigger is not on the roadmap.

The retry classifier (`is_terminal_error` in the dispatcher) makes
`TriggerError::Command { .. }`, `TriggerError::Template { .. }`,
`TriggerError::Webhook { status: Some(s), .. }` with `s.is_client_error()` (a
4xx HTTP response), and `TriggerError::WebhookBuild { .. }` (HTTP client builder
rejection: malformed URL, invalid header) all terminal under `fail_fast = true`
(the default) because they are deterministic: the same input produces the same
failure, so retrying wastes the budget. `Io`, `Encode`, `Timeout`, and
`TriggerError::Webhook` with a 5xx or `None` status stay retryable because they
are genuinely transient (broken pipe, disk transiently full, slow downstream,
server-side glitch, transport interrupt). `fail_fast = false` keeps every
failure retryable.

Checkpoint policy: a notification's `last_committed_sequence` advances **only**
after all required triggers succeed. The mechanism is the supervisor's existing
commit-on-next-send (the previous notification is committed just before the
current one is sent on the channel; the trigger pipeline runs between
commit-of-prev and send-of-current). Optional triggers may fail without blocking
the checkpoint; their failures log a `WARN` event with stable name
`client.trigger.failed`. If required triggers exhaust their retries the
supervisor surfaces `ClientError::TriggerFailed`, the stream terminates, and the
checkpoint stays where it was so restart re-delivers the notification whose
trigger failed.

*Amendment, 2026-05-22*: the ADR originally listed only echo and log; the
`command` trigger landed alongside the resurrected `Trigger::timeout` /
`Trigger::fail_fast` setters and a small template engine shared with the
eventual webhook trigger. The template engine intentionally renders objects and
arrays as compact JSON (not as a `NotString` error) so
`{{ notification.identifier }}` and `{{ notification.payload }}` are useful
directly. The `TriggerError::Template` variant carries a safe `context` label
rather than the raw template, addressing redaction concerns for command strings
that may include bearer tokens or connection URIs.

*Amendment, 2026-06-09*: the `webhook` trigger landed, completing the four-kind
set the ADR now enumerates. The webhook reuses the supervisor's shared
`reqwest::Client` (TLS configuration on the client inherits naturally), defaults
to `POST` with a compact-JSON notification body and
`Content-Type: application/json` when the user does not override either, and
defaults its per-trigger timeout to 30 seconds (exposed as the public
`DEFAULT_WEBHOOK_TIMEOUT`). URL, header values, and body all run through the
existing template engine; header names are taken literally. The retry classifier
extends to treat `TriggerError::Webhook` with a 4xx `status` as terminal under
`fail_fast = true` (the receiver is rejecting the request deterministically;
retrying does not help) and `TriggerError::WebhookBuild { reason }` as terminal
(the HTTP client rejected the rendered request at build time, e.g. malformed URL
after template render or invalid header value; the same notification will
produce the same rejection on retry); 5xx and `None` status (transport errors)
stay retryable as transient. Response body capture uses the streaming
`Response::chunk` primitive into a 4 KiB ring buffer so the captured-body-tail
storage stays capped at `RING_CAP` regardless of total body size streamed by the
server (per-chunk transient memory and the final lossy-UTF-8 decode are
proportional to chunk size, not body size, so the dispatcher's per-request
footprint stays bounded irrespective of how long the server keeps streaming).
The `TriggerKindLabel::Webhook` variant is bare (no body) for the same
secret-leak reason as `TriggerKindLabel::Command`: webhook URLs and header
values can carry tokens, and any visible-prefix summary risks leaking the secret
through error chains. The rendered URL and header values are deliberately NOT
surfaced through any error variant or default tracing event (a category-only
`client.trigger.webhook.send_failed` event with `is_timeout` / `is_builder` /
`is_connect` booleans is emitted at DEBUG without the reqwest `Display` string,
because reqwest's `Display` for builder and transport errors appends the
rendered URL unconditionally and the URL can hold secrets); operators who need
full request diagnostics extend their tracing layer with a custom subscriber.

The same amendment introduced the public `TriggerConfig` enum and its four
payload structs (`EchoConfig`, `LogConfig`, `CommandTriggerConfig`,
`WebhookTriggerConfig`) for declarative YAML configuration. The enum uses
tuple-payload variants with `#[serde(deny_unknown_fields)]` on each payload
struct rather than inline-struct variants on the outer enum: serde-rs issue
#2123 makes the outer-enum form unreliable for catching unknown fields within a
matched variant. Duration fields use `humantime_serde::option` so YAML strings
like `30s`, `2m`, `1h30m`, `500ms` round-trip cleanly. `HttpMethod` deserialises
from uppercase strings only (`POST`, `GET`, `PUT`, `PATCH`, `DELETE`); lowercase
fails loudly so a YAML typo is not silently accepted.

---

## D12. Logging per ECMWF Codex `Observability.md`

- Libraries (`aviso`, `aviso-py`) never configure global logging. They emit
  `tracing` events with stable `event.name` strings: `client.sse.*`,
  `client.resume.*`, `client.schema.*`, `client.auth.*`, `client.trigger.*`,
  `client.notify.*`.
- The CLI binary owns subscriber init: JSON to stderr by default in
  OpenTelemetry [logs data model][otel-logs] shape (top-level `timestamp`,
  `severityText`/`severityNumber`, `body`,
  `resource.{service.name,service.version[,deployment.environment]}`,
  `attributes.*`). Implemented as a custom `FormatEvent` in
  [`crates/aviso-cli/src/tracing_format.rs`](https://github.com/ecmwf/aviso-client/blob/main/crates/aviso-cli/src/tracing_format.rs).
  `traceId`/`spanId` are omitted because the CLI does not currently instrument
  distributed tracing; the OTel spec marks those `MUST when available`.
  Per-crate filter policy: `-v`/`-vv` raises the `aviso` crates to DEBUG/TRACE
  while every other crate (hyper, h2, reqwest, rustls) stays at WARN; operators
  who need transport-level diagnostics set `AVISO_LOG` explicitly (which
  overrides `-v` entirely and is treated as authoritative operator policy).

  [otel-logs]: https://opentelemetry.io/docs/specs/otel/logs/data-model/
- The Python extension bridges Rust *log records* to Python `logging` via
  `pyo3-log` with `Caching::LoggersAndLevels`, initialised once at module init.
  `pyo3-log` bridges the `log` crate, not `tracing` directly, so the chain is
  `tracing → tracing_log::LogTracer → log → pyo3-log → python logging`. An
  alternative is a custom `tracing_subscriber::Layer` that calls Python logging
  directly without the `log` hop; the binding work chooses based on whichever
  respects backpressure and the GIL better.
- Single-boundary log discipline: lower layers *return* structured errors; the
  reconnect supervisor logs classification and final outcome. On give-up, one
  primary `ERROR` carries the full error chain.
- Redaction at emission for known sensitive headers (`Authorization`, `Cookie`)
  and field names matching `password|token|secret|api_key`. URLs are sanitised
  (userinfo and sensitive query params removed). Request and response bodies are
  not logged by default; notification payloads are treated as sensitive unless
  explicitly enabled.
- Every log line carries `request_id` when known and `resume_key` in a watch
  context.

---

## D13. License Apache-2.0

The project is licensed under Apache-2.0 (`LICENSE.txt`) matching
`aviso-server`. Per-file ECMWF copyright headers are **not** required in v0.1;
they may be added in a single pass later if ECMWF software-publication policy
requires the boilerplate. Until then the LICENSE file plus the SPDX expression
in each `Cargo.toml` carries the licensing.

---

## D14. Tracing-only observability; metrics are a consumer concern

The client emits structured `tracing` events at every state transition
(connection open, connection close with reason, heartbeat starvation, replay
phase boundaries, trigger outcomes, fatal errors). See D12 for the stable
`event.name` strings and the redaction discipline. No `prometheus` or `metrics`
crate dependency ships in the core library, and there is no public metrics
surface.

Consumers that need Prometheus or OpenTelemetry metrics build a thin adapter on
top of the `tracing` events: a `tracing_subscriber::Layer` that maps specific
event names onto counter or histogram updates against the `metrics` crate facade
(or a direct Prometheus client). The adapter owns the policy decisions (which
events count, what labels apply, what aggregation window) that the library would
otherwise have to bake in for every consumer.

Rationale: a notification client emits the same handful of observability events
whether the operator wants Prometheus, StatsD, OpenTelemetry, or no metrics at
all. Wiring those events into a metrics facade inside the library forces every
consumer to pay the dependency cost. Keeping it out lets the core stay small and
lets each consumer pick their stack.

If a future adapter ships in this workspace or downstream, the recommended
Prometheus prefix is `aviso_client_` (underscore-separator with an explicit
`_client_` segment). The prefix disambiguates from `aviso-server`'s
`aviso_server_*` metrics when both are scraped into the same Prometheus
instance. The Rust crate name (`aviso`) and the recommended metric prefix
(`aviso_client_`) intentionally differ for this reason. Plausible counter and
histogram names, mapping onto the existing `tracing` events:

- `aviso_client_reconnects_total{reason,outcome}`
- `aviso_client_notifications_processed_total{outcome}`
- `aviso_client_trigger_duration_seconds{trigger_kind,outcome}`

Banned labels (ECMWF Codex high-cardinality rule), to carry forward into any
adapter: `request_id`, `resume_key`, raw URL, username, UUID, payload fields,
full event identifiers (`<event_type>@<sequence>`).

*Amendment, 2026-05-19*: this ADR originally read "Metrics deferred to v1.1",
framing a future client-side metrics surface with a reserved namespace and a
mechanical implementation path. The amendment reflects the current stance: the
core emits `tracing` events only, and metrics live in consumer-built adapters.
The Prometheus prefix and banned-labels guidance survive as forward advice for
any adapter crate.

---

## D15. Watch state machine: orthogonal product

See D2. `ReplayPhase × ConnectionStatus`, single reducer, fields private. Tests
must exercise invalid transition attempts through the public API and not by
mutating fields.

---

## D16. `notify()` is not auto-retried on ambiguous transport failure

A `POST /api/v1/notification` that fails with an ambiguous transport error after
the request body has been sent may have been processed by the server. The client
does not retry it. Failures before request-body transmission may be retried
where `reqwest` can classify them safely.

A server-side idempotency-key contract would lift this restriction; it is an
open ask, not a current dependency.

---

## D17. `from_date` is bootstrap-only

A user-supplied `from_date` is used only for the very first connection of a
listener. After the first notification is committed, the cursor switches to
sequence-based (`from_id = last_committed_sequence + 1`). Server time is
authoritative; the client does not attempt clock-skew compensation.

---

## D18. Accept `CDLA-Permissive-2.0` license for trust-root data

The Rust core uses `reqwest` with the `rustls-tls` feature (D5). Rustls's
default trust-root source is the
[`webpki-roots`](https://crates.io/crates/webpki-roots) crate, which ships
Mozilla's Common CA Database as embedded data. Mozilla licenses that data under
[CDLA-Permissive-2.0](https://cdla.dev/permissive-2-0/), a permissive data
license (free use and redistribution, no copyleft, no warranty). The license is
not on the workspace `deny.toml` allowlist by default.

**Decision**: add `CDLA-Permissive-2.0` to the `deny.toml` allowlist.

**Rationale**:

- The license is functionally similar to MIT/Apache-2.0 for data assets and is
  on the list of OSI-style permissive licenses.
- Bundled CA roots are operationally robust: they work in distroless and slim
  container images without a separate `ca-certificates` mount.
- Mozilla's CA programme is the de-facto trust root for the public web; using
  their curated bundle is the appropriate default for a generic HTTP client.

**Alternatives considered**:

- `rustls-tls-native-roots` (reads the system CA store via
  `rustls-native-certs`): smaller dep tree, no extra license, but fails in
  CA-less container images. Rejected for portability.
- Vendor a CA bundle ourselves: creates a maintenance burden tracking upstream
  Mozilla updates and offers no real benefit over `webpki-roots`. Rejected.

**Consequences**: any future workspace dependency that ships data under
`CDLA-Permissive-2.0` is permitted without further review. New non-permissive or
unusual licenses still require a fresh decision.

---

## D19. Watch API: a single `Stream` plus a handler-shaped sugar over the same channel

`AvisoClient::watch(request)` returns a `NotificationStream`, an async
`Stream<Item = Result<Notification, ClientError>>` backed by a bounded
`tokio::sync::mpsc::channel`. A second method `watch_with_handler(request, F)`
wraps the stream in a per-notification callback loop. Both surfaces drain the
exact same internal channel from the exact same supervisor task; the
supervisor's reconnect, checkpoint, auth-refresh, heartbeat, and trigger
behaviour is identical across both shapes.

**Rationale**:

- The Rust core has to bind cleanly into Python (PyO3, via `__aiter__` /
  `__anext__` over the same channel) and, at some future point, into a
  synchronous C/C++ adapter (via callback registration on the handler-shaped
  surface). A pure-stream-only API would force callback consumers to reimplement
  supervisor logic; a pure-callback-only API would deny native Rust users the
  idiomatic `Stream` shape.
- One supervisor task per `watch()` call keeps behaviour predictable for
  cross-language reviewers: there is a single source of truth for what the
  supervisor does, and the two surfaces just expose different drain ergonomics
  on the same channel.
- The bounded channel applies TCP backpressure end-to-end when the consumer
  falls behind. Capacity is fixed at 128 in the first iteration. A
  `WatchRequest::with_buffer` tuning knob is a future addition if a real
  consumer needs it.
- `NotificationStream` deliberately does **not** implement `Clone`. Fan-out
  across multiple consumers of a single watch would require a broadcast channel
  (or a tee), which conflicts with single-cursor checkpoint advancement: a
  "broadcast then commit" semantic has no obvious right answer when readers
  commit at different rates. Single-consumer keeps the checkpoint contract
  unambiguous. A caller that needs fan-out builds it with
  `tokio::sync::broadcast` themselves, on top of `NotificationStream::recv()`.

**Cancellation**: dropping the stream drops a `tokio::sync::oneshot::Sender`;
the supervisor `select!`s on the matching `Receiver` and exits cooperatively. No
`JoinHandle::abort`, no `tokio-util::CancellationToken` dependency, no
buffered-but-undelivered notifications. The `select!` uses `biased` ordering so
cancellation cannot be starved by a fast stream, and is wrapped around every
supervisor await (auth header, HTTP send, chunk read, channel send) so a drop
tears the supervisor down within one event-loop tick regardless of which await
it was parked on.

**Alternatives considered**:

- Stream-only public API. Rejected because the future C/C++ adapter would have
  to reimplement the stream-poll-and-callback loop in the FFI layer; doing it
  once in Rust is the correct factoring.
- Callback-only public API. Rejected because Rust users expect `Stream`,
  async-iterator integration with `tokio::select!`, and the ability to compose
  with the wider futures ecosystem.
- An unbounded internal channel. Rejected because a slow consumer would grow
  memory without bound; the bounded design applies backpressure where the
  underlying transport can react.
- A broadcast-channel-based `Clone`-able `NotificationStream`. Rejected because
  it conflicts with single-cursor checkpoint advancement; users who actually
  want fan-out can build it on top of the single-consumer primitive.

---

## D20. Multi-listener: an `AvisoClient` property, not a new primitive

A single `AvisoClient` supports any number of concurrent `watch()` calls. Each
call produces an independent supervisor task with its own HTTP connection, its
own checkpoint slot (once the state-store integration lands), and its own
trigger pipeline (once triggers land). `AvisoClient` is `Send + Sync + Clone`;
cloning it shares the inner `reqwest::Client`'s connection pool and the optional
`Arc<dyn AuthProvider>` through reference counts.

**Rationale**:

- Users who need to listen to multiple unrelated event types from one process
  spell that out as multiple `watch()` calls. The resume-key derivation (D3)
  guarantees per-listener isolation across distinct (event_type, filter,
  schema_fingerprint) tuples, so two listeners on the same client never alias
  their checkpoint slots in practice.
- Adding a hypothetical `Listeners` collection primitive would just be a thin
  wrapper around "call `watch()` N times". The wrapper would not change
  semantics; it would only add a public type that has to be kept compatible
  across versions. Keeping the primitive small and letting callers compose is
  the right factoring for a library that has to bind into multiple languages.
- Resource sharing happens at the layers where sharing is correct:
  `reqwest::Client` connection pool, `Arc<dyn AuthProvider>` for shared refresh
  state, `StateStore` for per-key write serialisation.
- Resume-key collision across overlapping `watch()` calls on the same client is
  a misuse signal, not a hard error. The follow-up that wires the state store
  will emit a single `WARN` log when a collision is detected; the supervisor
  continues either way and the caller learns from the log.

**Cancellation**: each supervisor is cancelled when its `NotificationStream` is
dropped. A parent-level cascade where dropping the `AvisoClient` cancels every
child supervisor is intentionally out of scope for the first iteration; the
design supports adding it later (a `tokio_util::sync::CancellationToken` or
equivalent owned by the client) because the supervisor task is constructed to
own only clones of the bits it needs (`reqwest::Client`, `Url`,
`Option<Arc<dyn AuthProvider>>`) rather than a cloned `AvisoClient` that would
keep the parent alive through refcount.

**Server-side capacity**: per-user and per-role connection or replay-rate limits
are server's problem, enforceable at the proxy or `aviso-server` layer using
existing JWT/Basic-auth identity. The client surfaces 429/503 via reconnect
classification (eventual exponential backoff once the reconnect loop lands), 401
via auth refresh, 403/404/410 as a terminal `Fatal(ProtocolViolation(...))`. The
client API does not change to accommodate server-side capacity policy.

**Live role revocation**: takes effect on the next reconnect, up to the server's
`connection_max_duration_sec` lag. Not modelled at the client level; a future
`ServerCloseReason::PermissionRevoked` could close the gap if the server ever
emits that variant.

**Alternatives considered**:

- A `Listeners` aggregate type that bundles a `Vec<NotificationStream>` and a
  single `recv_any()` method. Rejected for the reasons above: the primitive is
  already small enough that a generic wrapper offers no functional benefit and
  locks in a coupling between listeners that the channel-based design otherwise
  avoids.
- A single `AvisoClient` that owns one supervisor regardless of how many
  `watch()` calls happen. Rejected because that would require multiplexing all
  events into one stream and erasing the per-watch checkpoint distinction.

---

## Reference: server facts that drive these decisions

All verified from the `aviso-server` source. These constraints are why the ADRs
above look the way they do.

| Concern | Reality on the server | Client implication |
|---|---|---|
| SSE event format | `event: <type>\ndata: <json>\n\n`, no `id:` line | Standard `Last-Event-ID` reconnect does not apply. Sequence comes from the CloudEvent `id` field. |
| Resume parameters | `from_id` (inclusive sequence) and `from_date`, mutually exclusive, in the POST body | Persist `(stream_key, last_committed_sequence)`; reconnect with `from_id = last_committed_sequence + 1`. |
| Connection lifetime | Server closes after `connection_max_duration_sec` (default 3600 s), emitting `connection-closing` with `reason: "max_duration_reached"`. Other reasons: `server_shutdown`, `end_of_stream`. | Reconnects are routine, not failures. Must not trigger error backoff. |
| Heartbeats | `heartbeat` event every `sse_heartbeat_interval_sec` (default 30 s). | Absence beyond `max(3 x interval, interval + 30 s)` triggers a reconnect. |
| Event types | `live-notification`, `replay`, `heartbeat`, `connection-closing`, `error`, `replay-control` | Six frame kinds; the client uses a typed model. |
| Correlation | `X-Request-ID` on every response, also in the first SSE event and every error/control payload | Surfaced in client logs so users can quote it when filing issues. |
| Endpoints | `POST /api/v1/notification`, `POST /api/v1/watch`, `POST /api/v1/replay`, `GET /api/v1/schema[/{event_type}]`, `DELETE /api/v1/admin/{wipe/stream,wipe/all,notification/{id}}`, `GET /health`, `GET /metrics` (separate port) | Small, stable surface. |
| Auth | OpenAPI declares `bearer_jwt` and `basic`; `direct` mode forwards Basic to auth-o-tron, `trusted_proxy` mode validates Bearer JWT locally | Client supports both Basic and Bearer; sources composable (env, file, explicit). |
| Schemas | `GET /api/v1/schema` returns identifier rules per stream | Discovery only; no client-side validation (the server is the single source of truth). |
| Wire payload | CloudEvent JSON. Each notification has `id = <event_type>@<sequence>`, `source = base_url`, `type = int.ecmwf.aviso.<event_type>`, `time`, `data = {identifier, payload}` | Client hides the CloudEvent envelope; users see a `Notification`. Sequence is parsed from `id` via `rsplit_once('@')`. |

---

For the roadmap and follow-up tracking, see [`roadmap.md`](./roadmap.md) and
[`progress.md`](./progress.md).
