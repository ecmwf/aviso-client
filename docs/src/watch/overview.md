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

Both surfaces drain the same internal `tokio::sync::mpsc` channel from the same supervisor task. Reconnect, checkpoint, auth-refresh, heartbeat, and trigger behaviour is identical regardless of which surface you pick. The right choice is purely an ergonomic one:

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

The stream is single-consumer: it deliberately does NOT implement `Clone`. If you need a single watch to feed multiple consumers, wrap it in a `tokio::sync::broadcast` channel yourself. The single-consumer contract is a deliberate design choice; it keeps checkpoint advancement (when the state-store integration lands) unambiguous.

## Backpressure and channel capacity

The internal channel is bounded at 128 items. A slow consumer that lets the channel fill applies TCP backpressure all the way upstream: the supervisor's `send` `await`s, which makes it stop reading bytes from the wire, which throttles the server. Notifications are not dropped and the internal buffer does not grow without bound.

If a real consumer needs a different buffer size, the `with_buffer(n)` knob is a future addition; the fixed-128 default is intentionally conservative.

## Cancellation and drop

Dropping the `NotificationStream` cancels the supervisor cooperatively. Mechanism: the stream owns one half of a `tokio::sync::oneshot` channel, and the supervisor's `select!` loop watches the other half. Dropping the stream drops the sender, which the supervisor observes within one event-loop tick. No `JoinHandle::abort`, no buffered-but-undelivered notifications, no leaked task.

Cancellation is responsive in every supervisor state: while awaiting auth, while sending the initial request, while reading chunks from the wire, and while blocked on a full channel from a slow consumer. The `select!` uses `biased` ordering so a fast stream cannot starve cancellation.

For `watch_with_handler`, returning `Err(_)` from the handler drops the stream at the end of the loop, which cancels the supervisor the same way.

## Errors

`ClientError` variants you can see on a watch stream:

- `Http { status, body, request_id }` on the initial response if the server returned a non-success status. Only this variant comes from the call to `watch()` itself; everything else surfaces on the stream.
- `Transport(_)` on mid-stream transport failure (TLS error, connection reset, and so on).
- `Decode(_)` on a wire-shape JSON payload that does not deserialise. Terminal.
- `MalformedEvent(_)` on a CloudEvent whose `id` field does not parse as `<event_type>@<u64>`. Terminal per the same reasoning as the rest of the client: a poisoned stream would otherwise livelock the reconnect loop.
- `HistoryGap { reason }` when the supervisor detects either a non-consecutive sequence number on the wire or a server-emitted replay-limit signal. Terminal: continuing past a known gap would silently violate at-least-once delivery.
- `StreamProtocol { message, request_id }` for the server's `error` SSE event and for `connection-closing` frames whose `reason` is not one of the three documented values. The `request_id` carries the server-supplied correlation id when the payload includes one; quote it when filing issues.
- `Config(_)` from `watch()` itself: no Tokio runtime is entered, or the resume position overflows `u64::MAX`.

## Multiple listeners on one client

`AvisoClient` is `Send + Sync + Clone`. A single client supports any number of concurrent `watch()` calls; each runs its own supervisor task and its own HTTP connection. Resource sharing happens cleanly at the right layers: the reqwest connection pool is shared, the auth provider is shared via `Arc`, and (in a future follow-up) per-resume-key writes to the state store are serialised internally.

```rust,ignore
let mars_stream = client.watch(WatchRequest::watch("mars"))?;
let cosmo_stream = client.watch(WatchRequest::watch("cosmo"))?;
// Both streams are live independently.
```

When the parent `AvisoClient` is dropped, all of its child supervisor tasks are cancelled (the follow-up resilience work wires this; the design supports it because supervisors own clones of the bits they need rather than a cloned `AvisoClient` that would keep the parent alive by refcount).

## What this version does not do

- **Reconnect.** When the server's `connection_max_duration_sec` elapses, the supervisor exits the single connection cleanly. A reconnect loop driven by the watch state machine's `ReconnectPolicy` is the next major piece of work.
- **Heartbeat watchdog.** Heartbeat events are received and acknowledged, but a long silence from the server does not yet trigger a reconnect.
- **Auth refresh on `401`.** A 401 on the initial response surfaces as `ClientError::Http`; the supervisor does not yet drive `AuthProvider::refresh` and retry.
- **Checkpoint advancement to a `StateStore`.** The supervisor does not yet persist `last_committed_sequence` between sessions. Until that lands, callers pass an explicit `ResumeStart` per `watch()` call.

Together with these in the next iteration, a watch session survives an arbitrary number of routine server-driven reconnects transparently.

## Architectural references

- `docs/src/internals/decisions.md` D2 (reconnect-as-norm, state machine sketch), D9 (CloudEvent envelope hidden, malformed id terminal), D15 (state machine as `ReplayPhase x ConnectionStatus`), D17 (`from_date` bootstrap-only), D19 (watch API shape and single-consumer mpsc), D20 (multi-listener as `AvisoClient` property).
