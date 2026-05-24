# Watch streams

`aviso-server` delivers notifications over a long-lived SSE stream behind the `POST /api/v1/watch` and `POST /api/v1/replay` endpoints. The client wraps that stream in a small async surface: one method that returns an async `Stream`, plus a thin callback wrapper for daemon-style consumers. Both share the same internal supervisor.

## Two surfaces, one supervisor

```rust,ignore
use aviso::{AvisoClient, Notification, Result};
use aviso::watch::{NotificationStream, ResumeStart, WatchRequest};
use futures_core::Stream;

// Stream surface: idiomatic async iteration. Drop the stream to cancel.
let stream: NotificationStream = client.watch(WatchRequest::watch("mars"))?;

// Callback surface: per-notification handler loop with the same semantics.
client
    .watch_with_handler(WatchRequest::watch("mars"), |notification| async move {
        println!("got sequence {}", notification.sequence);
        Ok(())
    })
    .await?;
```

Both surfaces drain the same internal `tokio::sync::mpsc` channel from the same supervisor task. The single-connection behaviour is identical regardless of which surface you pick. The right choice is purely an ergonomic one:

- `watch()` for `tokio::select!` integration, composition with other futures, or any case where you want the raw `Stream`.
- `watch_with_handler()` for write-only daemon loops that process notifications in isolation.

## Building a `WatchRequest`

Three constructors make the invalid shape unrepresentable:

```rust,ignore
use std::collections::BTreeMap;
use serde_json::json;
use aviso::watch::{ResumeStart, WatchRequest};

// Live-only: server streams new notifications immediately.
let req = WatchRequest::watch("mars");

// Historical-then-live: replay everything after sequence 41, then continue live.
let req = WatchRequest::watch_from("mars", ResumeStart::AfterSequence(41));

// Replay-only: replay from a date, then close on end_of_stream.
let req = WatchRequest::replay_only(
    "mars",
    ResumeStart::Date("2026-01-01T00:00:00Z".to_string()),
);

// Add a filter. Values are JSON, not just strings, so spatial and
// range constraints fit alongside scalar identifiers.
let mut filter = BTreeMap::new();
filter.insert("country".to_string(), json!("UK"));
filter.insert("polygon".to_string(), json!({"type": "polygon", "points": [[0,0],[1,1]]}));
let req = WatchRequest::watch("mars").with_filter(filter);
```

The constructor selects the right server endpoint internally (`watch`/`watch_from` -> `/api/v1/watch`; `replay_only` -> `/api/v1/replay`). A replay-only request always carries a resume position; the type system forbids the "replay-only with no start" combination.

`ResumeStart::AfterSequence(n)` reads "I already have everything up to and including sequence `n`; give me `n+1` onward". The supervisor sends `from_id = (n + 1).to_string()` on the wire. `ResumeStart::Date(s)` is sent verbatim as `from_date`. The two are mutually exclusive (server-enforced; the client also rejects ill-formed combinations at request-build time).

## The stream contract

`NotificationStream` is an async `Stream<Item = Result<Notification, ClientError>>`. The supervisor produces these items in order:

- `Ok(Notification)` for each notification the server delivered. `event_type` and `sequence` come from the CloudEvent `id`; `identifier` is the schema identifier map; `payload` is the JSON payload (or `Value::Null` if absent).
- The first `Err(_)` is terminal. The stream yields `None` on the next call. The supervisor task has already exited; there is nothing more to read.
- `None` without an error means the stream ended cleanly: the server closed the connection, the consumer dropped the stream, or a replay-only session ran to completion.

The stream is single-consumer: it deliberately does NOT implement `Clone`. If you need a single watch to feed multiple consumers, wrap it in a `tokio::sync::broadcast` channel yourself. The single-consumer contract is a deliberate design choice; it keeps checkpoint advancement (with the state-store integration in place) unambiguous.

## Backpressure and channel capacity

The internal channel is bounded at 128 items. A slow consumer that lets the channel fill applies TCP backpressure all the way upstream: the supervisor's `send` `await`s, which makes it stop reading bytes from the wire, which throttles the server. Notifications are not dropped and the internal buffer does not grow without bound.

If a real consumer needs a different buffer size, the `with_buffer(n)` knob is a future addition; the fixed-128 default is intentionally conservative.

## Cancellation and drop

Dropping the `NotificationStream` cancels the supervisor cooperatively. Mechanism: the stream owns one half of a `tokio::sync::oneshot` channel, and the supervisor's `select!` loop watches the other half. Dropping the stream drops the sender, which the supervisor observes within one event-loop tick. No `JoinHandle::abort`, no buffered-but-undelivered notifications, no leaked task.

Cancellation is responsive in every supervisor state: while awaiting auth, while sending the initial request, while reading chunks from the wire, and while blocked on a full channel from a slow consumer. The `select!` uses `biased` ordering so a fast stream cannot starve cancellation.

For `watch_with_handler`, returning `Err(_)` from the handler drops the stream at the end of the loop, which cancels the supervisor the same way.

## Errors

`ClientError` variants you can see while running a watch:

- `Config(_)` is returned synchronously from `watch()` itself when no Tokio runtime is entered, when a user-supplied resume position (`WatchRequest::watch_from(..., ResumeStart::AfterSequence(n))`) overflows `u64::MAX`, or when the resume-key derivation rejects the filter shape. A `u64::MAX` overflow originating from a STORED checkpoint (loaded asynchronously by the supervisor from a configured `StateStore`) cannot surface synchronously; it appears instead as the first stream item, also as `Err(Config(_))`, and indicates invalid or corrupted stored state that the operator must repair (typically by deleting the affected entry).

The resilience layer absorbs every retryable failure mode internally. The supervisor reconnects with exponential backoff on transport errors, on TCP EOF without a `connection-closing` frame, on heartbeat starvation, and on `429`/`503` HTTP statuses (honouring `Retry-After` when present, capped at five minutes). None of these surface as stream items; you will not see a transient `Transport(_)` or a 503 as a terminal error.

The remaining error variants are terminal: they surface as `Some(Err(_))` followed by `None`, and the supervisor has exited:

- `Http { status, body, request_id }` for `403`, `404`, `410`, and any other non-2xx-non-retryable status, with the verbatim server response preserved. A `401` ALSO lands here when no `AuthProvider` is configured on the client (the supervisor has no refresh cycle available, so it surfaces the 401 directly rather than retrying); when an auth provider IS configured, the supervisor instead runs the refresh-then-retry-once cycle and only surfaces a 401 as `Auth(_)` (see next bullet) after the refreshed credential is also rejected.
- `Auth(_)` when an `AuthProvider::refresh()` call returns an error, or when a second 401 arrives within the same attempt cycle (signalling the refreshed credential was also rejected).
- `StreamProtocol { message, request_id }` for the server's `error` SSE event and for `connection-closing` frames whose `reason` is not one of the three documented values. The `request_id` carries the server-supplied correlation id when the payload includes one; quote it when filing issues.
- `StateStore(_)` when a configured `StateStore` fails during `get` or `put`. Continuing without a working store would silently violate at-least-once delivery, so this is terminal.
- `TriggerFailed { kind, source }` when a required trigger fails after all configured retries. The committed checkpoint stays at the previous notification's sequence; on next process start the supervisor re-delivers the notification whose trigger failed. `kind` is `TriggerKindLabel::Echo`, `TriggerKindLabel::Log { path }`, or `TriggerKindLabel::Command` (the bare command label carries no body; the full command may contain secrets and lives in DEBUG tracing only). `source` covers the full `TriggerError` axis: `Io`, `Encode`, `Command { exit_code, stderr_tail }`, `Template { context, field, kind }`, or `Timeout(Duration)`.
- `Decode(_)` on a wire-shape JSON payload that does not deserialise.
- `MalformedEvent(_)` on a CloudEvent whose `id` field does not parse as `<event_type>@<u64>`. Terminal to avoid livelocking on a poisoned stream.
- `HistoryGap { reason }` when the supervisor detects either a non-consecutive sequence number on the wire or a server-emitted replay-limit signal. Terminal: continuing past a known gap would silently violate at-least-once delivery.

## Multiple listeners on one client

`AvisoClient` is `Send + Sync + Clone`. A single client supports any number of concurrent `watch()` calls; each runs its own supervisor task and its own HTTP connection. Resource sharing happens cleanly at the right layers: the reqwest connection pool is shared, the auth provider is shared via `Arc`, and per-resume-key writes to the state store (when configured) are serialised inside the `StateStore` implementation.

```rust,ignore
let mars_stream = client.watch(WatchRequest::watch("mars"))?;
let cosmo_stream = client.watch(WatchRequest::watch("cosmo"))?;
```

Two cancellation paths exist:

- **Per-stream**: dropping a `NotificationStream` cancels its supervisor cooperatively via the oneshot mechanism documented above.
- **Parent-cascade**: dropping the LAST `AvisoClient` clone (the underlying `Arc<DropGuard>` reference) fires a `tokio::sync::watch` flip that every child supervisor observes. Both streams in the snippet above terminate within one event-loop tick of the client drop (or after a state-store `put` in progress completes, per the cancel-safety contract).

If two concurrent `watch()` calls on the same client compute the same resume key (same base URL + same event type + same filter), the client emits one `WARN` tracing event with `event.name = "client.resume.collision"` carrying the hex digest and the event type. Checkpoint advancement is racy in that case; the warning is the diagnostic, not an error. The supervisor continues; the caller decides whether to deduplicate the watches.

## Reconnect-as-norm

`aviso-server` deliberately closes connections after `connection_max_duration_sec` (default 3600 s). The supervisor reconnects automatically; a watch outlives an arbitrary number of routine server-driven reconnects without ever surfacing a terminal error.

The reconnect classifier:

- `connection-closing { reason: max_duration_reached }`: immediate reconnect, no backoff. The routine path.
- `connection-closing { reason: server_shutdown }`: short backoff (~5 s) then reconnect.
- `connection-closing { reason: end_of_stream }` in `WatchMode::Watch`: immediate reconnect.
- `connection-closing { reason: end_of_stream }` in `WatchMode::ReplayOnly` after the server has emitted `replay_completed`: terminal. The replay finished naturally; the stream closes cleanly with `None`.
- TCP EOF without a `connection-closing` frame: reconnect with exponential backoff (NAT timeouts, half-open sockets after laptop sleep, intermediate-proxy restarts).
- Mid-stream transport error (TLS, peer reset, read error): reconnect with exponential backoff.

Backoff: AWS-style full jitter, base 250 ms doubling per attempt, capped at 30 s. Random source is in-process (no `rand` dependency) and not cryptographic; the goal is to spread a fleet's retries across a window so a server-side outage does not cause a thundering herd on recovery.

## Heartbeat watchdog

The supervisor wraps each chunk read in `tokio::time::timeout`. If no SSE event of any kind (heartbeat, notification, or control) arrives within `max(3 * heartbeat_interval, heartbeat_interval + 30s)`, the supervisor declares the connection silently dead and reconnects with exponential backoff.

The default `heartbeat_interval` is 30 s, matching the default `aviso-server` configuration. Override via the builder when targeting a server with a non-default heartbeat cadence:

```rust,ignore
use std::time::Duration;

let client = AvisoClient::builder()
    .base_url("https://aviso.example.org")
    .heartbeat_interval(Duration::from_secs(10))
    .build()?;
```

The watchdog catches failure modes that TCP keepalive does not:

- NAT idle timeout (typical: 30 min to a few hours on consumer routers, 1 hour on many corporate firewalls).
- Half-open sockets after network change (laptop sleep, WiFi roam, VPN reconnect).
- Server-side application hang behind a healthy reverse proxy (the proxy's TCP stack ACKs keepalive probes; the upstream is gone).

The 30 s absolute floor in the budget formula prevents the watchdog from tripping during transient network slowness.

## Auth refresh

`AvisoClientBuilder::auth(Arc<dyn AuthProvider>)` configures an auth provider. On a 401 response, the supervisor:

1. Fires `WatchEvent::AuthRejected` and the reducer transitions to `RefreshingAuth`.
2. Calls `auth.refresh().await`. The refresh is in a `tokio::select!` with both cancel arms, so a stream drop during refresh observes cancellation promptly.
3. On `Ok(())`: reconnect with the refreshed credential. If the server returns 401 again within the same attempt cycle (no successful intervening response), the watch terminates with `ClientError::Auth("authentication rejected after refresh")`.
4. On `Err(_)`: surface the auth provider's error and terminate.

The "refresh-then-retry-once" cycle resets after any non-401 outcome (a successful connection, a routine server close, a transport error, a heartbeat-starved reconnect). Long-lived watches that span token-rotation events naturally refresh many times across their lifetime; only consecutive 401s within a single attempt cycle terminate.

## State-store-backed resume

`AvisoClientBuilder::state_store(Arc<dyn StateStore>)` wires a persistent store (`MemoryStore`, `JsonFileStore`, or a custom implementation) into the client. When set:

1. At watch start, if `WatchRequest::from()` is `None`, the supervisor reads the stored checkpoint and resumes from `last_committed_sequence + 1` on the wire.
2. After each successful notification send, the supervisor persists the PREVIOUS notification's sequence and event id before the next send leaves the channel. The user-facing contract is "pulling item N+1 implies item N is durable", and the client enforces this by forcing the internal channel capacity to 1 when a `StateStore` is configured. With capacity 1, the supervisor's send of item N+1 blocks (TCP backpressure propagates upstream) until the consumer has pulled item N; the supervisor's commit of N therefore happens before the consumer can ever pull N+1. Without a `StateStore`, capacity stays at the default 128 and there is no durability claim to enforce.
3. A user-supplied `from` always wins. The stored checkpoint is consulted only when the request has no explicit resume position.

```rust,ignore
use std::sync::Arc;
use aviso::state::{JsonFileStore, StateStore};

let store: Arc<dyn StateStore> = Arc::new(JsonFileStore::open("~/.config/aviso/state.json")?);
let client = AvisoClient::builder()
    .base_url("https://aviso.example.org")
    .state_store(store)
    .build()?;
```

A failure to persist the checkpoint terminates the watch with `ClientError::StateStore(_)`. Continuing without a working store would silently violate at-least-once delivery; the user must see the failure to fix the underlying problem (disk full, file corrupted, permissions).

## Triggers

A trigger is a per-notification side effect attached to a `WatchRequest` via `with_triggers(Vec<Trigger>)`. Triggers run in declaration order **before** each notification is sent on the consumer channel, so the consumer never sees an item whose required triggers have not all passed. Four built-in kinds ship in the core:

- `Trigger::echo()` writes the notification as a single compact JSON line to standard output (NDJSON; one line per notification, terminating `\n`). The trigger has no configurable destination; it writes to `stdout` via a locked handle. Use `aviso watch ... | jq` to compose with line-oriented tools.
- `Trigger::log(path)` appends the same NDJSON line to a user-specified file. The file is opened on first dispatch with `append(true).create(true)` and held open for the watch's lifetime; the trigger does not `fsync` per write (at-least-once at the state-store level covers crash replay), does not rotate the file (use `logrotate` or similar externally), and does not reopen on signals.
- `Trigger::command(cmd)` runs `/bin/sh -c <rendered>` per notification, with the notification's fields exposed as `AVISO_*` environment variables. The command string is rendered through a small template engine (`{{ notification.<dotted.path> }}` substitutes a notification field, `{{ env.<NAME> }}` reads from the process environment). Unix-only.
- `Trigger::webhook(url)` sends an HTTP request per notification to a user-configured URL. URL, header values, and body all run through the same template engine; header names are taken literally. Default method is `POST`, default body is the notification serialised as compact JSON (same shape as `echo`), default `Content-Type` is `application/json` when the user does not set one, default per-trigger timeout is 30 seconds. Cross-platform.

```rust,ignore
use std::time::Duration;
use aviso::watch::{HttpMethod, Trigger, WatchRequest};

let req = WatchRequest::watch("mars")
    .with_triggers(vec![
        Trigger::echo(),
        Trigger::log("/var/log/aviso/notifications.log"),
        Trigger::command("./process.sh {{ notification.event_type }}")
            .env("EXTRA_KEY", "value")
            .working_dir("/var/data/aviso")
            .timeout(Duration::from_secs(60)),
        Trigger::webhook("https://hooks.example.org/notify")
            .method(HttpMethod::Post)
            .header("Authorization", "Bearer {{ env.HOOK_TOKEN }}")
            .body_template(r#"{"event": "{{ notification.event_type }}", "seq": {{ notification.sequence }}}"#)
            .timeout(Duration::from_secs(10))
            .retries(3),
    ]);
let mut stream = client.watch(req)?;
```

Each trigger has four tunables on the builder:

- `.retries(u32)` (default `0`): up to `N` additional attempts after the initial-attempt failure, for a total of `N + 1` attempts. Backoff between attempts is the supervisor's standard exponential schedule with full jitter (base 250 ms, capped at 30 s).
- `.required(bool)` (default `true`): a required trigger that fails after all retries terminates the watch with `ClientError::TriggerFailed`; an optional trigger that fails after all retries emits a `tracing::warn!` event with stable name `client.trigger.failed` carrying `kind`, `retries`, and the inner error display, and the watch continues to the next trigger and the channel send.
- `.timeout(Duration)` (default `None` for echo, log, and command; `DEFAULT_WEBHOOK_TIMEOUT` (30 s) for the HTTP-based triggers webhook, teams, and post): per-attempt timeout for the command trigger and the HTTP-based triggers. On expiry the command dispatcher kills the shell child and the HTTP dispatchers drop the in-flight request; both return `TriggerError::Timeout(t)`. Echo and log silently ignore the field: each one writes a buffer-prepared NDJSON line in a single I/O call (locked-stdout `write_all` for echo, `tokio::fs::File::write_all` for log), and the dispatcher preserves that single-call atomicity rather than racing the write against a cancellable sleep that could leave a truncated or malformed line on stdout or in the log file.
- `.fail_fast(bool)` (default `true`): treats deterministic failures as terminal so the retry budget is not wasted on them. For command triggers: `TriggerError::Command` (non-zero exit) and `TriggerError::Template`. For the HTTP-based triggers (webhook, teams, post): `TriggerError::Webhook` with a 4xx HTTP status (the receiver is rejecting the request), `TriggerError::WebhookBuild` (HTTP client rejected the rendered request at build time, e.g. malformed URL or invalid header value), and `TriggerError::Template`. `Io`, `Encode`, `Timeout`, and `TriggerError::Webhook` with a 5xx or `None` status (server-side glitch or transport error) stay retryable because they are transient. Echo and log silently ignore the field (their errors are always retryable).

```rust,ignore
let req = WatchRequest::watch("mars").with_triggers(vec![
    Trigger::log("/var/log/aviso/audit.log"),                  // required
    Trigger::log("/var/log/aviso/telemetry.log")
        .retries(3)
        .required(false),                                      // optional
    Trigger::command("./consume.sh {{ notification.identifier.country }}")
        .timeout(std::time::Duration::from_secs(30))
        .retries(2),
    Trigger::webhook("https://hooks.slack.com/services/{{ env.SLACK_HOOK }}")
        .body_template(r#"{"text": "Got notification {{ notification.sequence }}"}"#)
        .retries(2),
]);
```

### Webhook retry semantics

The webhook dispatcher classifies HTTP responses for retry purposes:

- **2xx**: success; the trigger attempt returns `Ok(())`.
- **4xx**: terminal under `fail_fast = true` (the default). The receiver is rejecting the request deterministically (`400 Bad Request`, `401 Unauthorized`, `404 Not Found`, ...); retrying will not change the outcome. Under `fail_fast = false`, every error is retried through the full budget.
- **5xx**: retryable, even under `fail_fast = true`. Server-side glitches typically resolve on retry; the budget exists for exactly this case.
- **Transport error** (DNS, TCP, TLS, mid-response interrupt): retryable. `TriggerError::Webhook { status: None, body_tail: String::new() }`.

The dispatcher captures the last 4 KiB of the response body into a bounded ring buffer using the streaming `Response::chunk` primitive, so a misbehaving server cannot OOM the client with an oversized error body. The body tail surfaces in `TriggerError::Webhook { body_tail, .. }` for operator diagnosis.

### Declarative YAML configuration

Trigger lists round-trip from YAML through the `TriggerConfig` enum and its payload structs. The CLI and the Python bindings both consume this:

```yaml
triggers:
  - type: echo
    retries: 0

  - type: log
    path: /var/log/aviso/notifications.log
    retries: 2

  - type: command
    command: "./process.sh {{ notification.event_type }}"
    env:
      EXTRA_KEY: value
    working_dir: /var/data/aviso
    timeout: 60s
    retries: 1

  - type: webhook
    url: "https://hooks.example.org/notify"
    method: POST
    headers:
      Authorization: "Bearer {{ env.HOOK_TOKEN }}"
    body_template: '{"event": "{{ notification.event_type }}", "seq": {{ notification.sequence }}}'
    timeout: 10s
    retries: 3
```

Duration values use the [humantime](https://docs.rs/humantime) syntax (`30s`, `2m`, `1h30m`, `500ms`). HTTP method must be uppercase (`POST`, `GET`, `PUT`, `PATCH`, `DELETE`); a lowercase method or an unknown method name fails YAML deserialisation with a clear error. Each variant rejects unknown fields, so a misspelled key fails loudly rather than being silently ignored.

### Atomicity and ordering

For echo and log triggers, each dispatch attempt is the atomic unit: the supervisor serialises the notification into a `Vec<u8>` once (with the newline appended to the same buffer) and writes the buffer through a single `write_all` call. Within a single dispatch attempt the dispatcher does not race against cancellation, matching the same atomicity contract that applies to `StateStore::put`. Between attempts (during retry backoff sleep) and between triggers (in the per-notification loop), the dispatcher honours both parent-drop and the per-stream cancel oneshot.

For the command trigger, the unit of work is one child-process spawn plus its `wait()`. When a timeout fires the dispatcher issues `SIGKILL` to the `/bin/sh -c ...` child, reaps the zombie, and returns `TriggerError::Timeout(t)`. The kill signal does NOT propagate to pipelines, backgrounded jobs, or grandchildren that survive the shell; operators who need full process-tree cleanup should use `exec ./binary` so the shell PID equals the target binary's PID. A 5-second post-exit drain cap bounds the dispatcher's exposure to descendants holding the pipes open after the shell exits.

For the webhook trigger, the unit of work is one HTTP request from `send()` through `chunk()`-drained body capture. The dispatcher uses reqwest's `RequestBuilder::timeout(t)` to bound the wait; on expiry reqwest drops the in-flight request and the dispatcher returns `TriggerError::Timeout(t)`. The body capture streams chunks into a bounded ring buffer; the captured body-tail storage is capped at 4 KiB regardless of total body size streamed by the server, so a server that responds with a multi-gigabyte body never grows the body-tail buffer past that cap. Per-chunk transient memory (the `bytes::Bytes` reqwest hands to `chunk()`, the lossy-UTF-8 expansion when the final tail is decoded into a `String`) is proportional to chunk size, not body size, so the dispatcher's webhook memory footprint stays bounded irrespective of how long the server keeps streaming.

The pipeline runs **before** the channel send, so a required trigger that fails for notification N never causes N to be sent or committed. On the next process start, the supervisor's resume cursor still points at the previous notification's sequence and N is re-delivered.

### Limitations

- The command trigger is POSIX-only. The `Trigger::command` API and the related `TriggerKindLabel::Command` / `TriggerError::Command` variants are gated behind `#[cfg(unix)]`; Windows builds of the crate compile cleanly without them and the echo / log / webhook triggers continue to work.
- No log rotation, no signal-driven reopen, no per-write `fsync` on the log trigger. External tools like `logrotate` handle rotation; crash replay covers durability via the state-store's at-least-once invariant.
- The command-trigger kill on timeout reaches only the `/bin/sh -c ...` child, not descendants. Use `exec` or accept the documented 5-second post-exit drain cap.
- The webhook trigger reuses the supervisor's shared `reqwest::Client`. Per-trigger HTTP client construction (custom TLS config, custom connection pool tuning, custom proxy) is not exposed in v1; configure those at the `AvisoClient` level.
- No multipart bodies, no webhook signing (HMAC-SHA256 or Stripe-style timestamp+signature). Operators who need either of these go through the command trigger and `curl`.
- No process-level stdout capture in tests. Echo content is verified by unit tests over the serialisation; integration tests verify the pipeline does not break stream delivery.

## Operator ingress recipe

For deployments behind a Kubernetes ingress, the watch endpoint needs three annotations to keep the long-lived SSE stream healthy. Verified working with the `nginx.org` ingress controller:

```yaml
annotations:
  nginx.org/proxy-buffering: "false"
  nginx.org/proxy-read-timeout: 3600s
  nginx.org/proxy-send-timeout: 3600s
```

The buffering knob is mandatory: without it nginx buffers the SSE response body until "complete" and the client receives nothing until the connection closes. The two timeouts must meet or exceed the server's `connection_max_duration_sec` so nginx does not impose an earlier cutoff than the server.

Operators using a different ingress controller (HAProxy, Traefik, Envoy, AWS ALB) should look up the equivalent of these three knobs; the heartbeat watchdog defends against silent failures across any ingress, but minimising routine reconnects keeps the operational picture cleaner.
