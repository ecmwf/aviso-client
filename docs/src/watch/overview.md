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

- `Config(_)` is returned synchronously from `watch()` itself when no Tokio runtime is entered, when the resume position would overflow `u64::MAX`, or when the resume-key derivation rejects the filter shape.

The resilience layer absorbs every retryable failure mode internally. The supervisor reconnects with exponential backoff on transport errors, on TCP EOF without a `connection-closing` frame, on heartbeat starvation, and on `429`/`503` HTTP statuses (honouring `Retry-After` when present, capped at five minutes). None of these surface as stream items; you will not see a transient `Transport(_)` or a 503 as a terminal error.

The remaining error variants are terminal: they surface as `Some(Err(_))` followed by `None`, and the supervisor has exited:

- `Http { status, body, request_id }` for `403`, `404`, `410`, and any other non-2xx-non-retryable status, with the verbatim server response preserved.
- `Auth(_)` when an `AuthProvider::refresh()` call returns an error, or when a second 401 arrives within the same attempt cycle (signalling the refreshed credential was also rejected).
- `StreamProtocol { message, request_id }` for the server's `error` SSE event and for `connection-closing` frames whose `reason` is not one of the three documented values. The `request_id` carries the server-supplied correlation id when the payload includes one; quote it when filing issues.
- `StateStore(_)` when a configured `StateStore` fails during `get` or `put`. Continuing without a working store would silently violate at-least-once delivery, so this is terminal.
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

The reconnect classifier (per ADR D2):

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
2. After each successful notification send, the supervisor persists the PREVIOUS notification's sequence and event id before the next send leaves the channel. Pulling item N+1 implies item N is durable.
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

## Architectural references

- `docs/src/internals/decisions.md` D2 (reconnect-as-norm, state machine sketch, reconnect classifier), D3 (resume key derivation), D4 (`StateStore` trait), D8 (`AuthProvider` and refresh-on-401), D9 (CloudEvent envelope hidden, malformed id terminal), D15 (state machine as `ReplayPhase x ConnectionStatus`), D17 (`from_date` bootstrap-only), D19 (watch API shape and single-consumer mpsc), D20 (multi-listener as `AvisoClient` property).
