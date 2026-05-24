# Architecture

A bird's-eye view of how aviso is built.

## The four crates

```text
+--------------------------------------------------------------------+
|  consumers                                                         |
|                                                                    |
|   +--------------------+      +--------------------------------+   |
|   |  aviso-cli         |      |  aviso (Python package, when   |   |
|   |  (Rust binary,     |      |   it ships)                    |   |
|   |   installed as     |      |    +-------------------------+ |   |
|   |   `aviso`)         |      |    |  aviso-py (PyO3 cdylib  | |   |
|   +---------+----------+      |    |   once bindings land)   | |   |
|             |                 |    +-----------+-------------+ |   |
|             |                 +----------------+----------------+   |
|             |                                  |                    |
|             +-----------------+----------------+                    |
|                               |                                     |
|                               v                                     |
|              +---------------------------------------+              |
|              |  aviso  (core Rust library)           |              |
|              |   - HTTP via reqwest + rustls         |              |
|              |   - SSE parser (finesse crate)        |              |
|              |   - Reconnect supervisor              |              |
|              |   - State store + checkpoints         |              |
|              |   - AuthProvider trait                |              |
|              |   - Trigger dispatcher                |              |
|              +-------------------+-------------------+              |
+----------------------------------+-----------------------------------+
                                   |
                                   v
                          HTTP + SSE over the wire
                                   |
                                   v
                          aviso-server (separate repo)
```

The CLI and the future Python extension are peer consumers of the core library. They never depend on each other. The core library never depends on PyO3, the CLI, or any binding machinery.

Adding a new language surface (a C/C++ adapter, for example) becomes a new adapter crate next to `aviso-cli` and `aviso-py`. The core library does not change.

## The crates in more detail

### `aviso` (core library)

The whole client lives here. Everything else is a thin wrapper.

- **HTTP**: `reqwest` with `rustls-tls`. Matches the server's choice.
- **SSE**: an in-tree parser (`finesse`) implementing the WHATWG parsing algorithm. The reconnect loop is owned, not delegated; off-the-shelf SSE crates assume the WHATWG `Last-Event-ID` mechanism, but aviso-server's resume contract is `from_id`/`from_date` in the POST body, which requires a re-POST on every reconnect.
- **Reconnect supervisor**: a single task per `watch()` call, driving a small state machine.
- **State store**: a trait with two built-in implementations, an in-memory store and a JSON file store with crash-safe atomic writes.
- **AuthProvider**: an async trait with five built-in providers.
- **Trigger dispatcher**: a crate-private enum with a public builder API.

### `aviso-cli`

The thin binary on top. Resolves the layered config (flag > env > file > default), builds an `AvisoClient`, wires in a `JsonFileStore`, parses listener YAML, dispatches subcommands.

The CLI's responsibilities are mostly about composition and I/O surfaces. It does not implement any of the SSE, reconnect, or trigger logic; that is all in the library.

### `aviso-py`

A placeholder today (`rlib`, not a Python `cdylib`). When the PyO3 bindings land it becomes the extension crate. It will expose an `AvisoClient` mirror plus a `NotificationStream` async iterator over the same channel the Rust API uses.

### `finesse`

The SSE parser, kept separate so the protocol can evolve without churning the rest of the library. Not published; reused across other workspace consumers if needed.

## Data flow at runtime

When you call `client.watch(WatchRequest::watch("mars"))`:

1. The watch surface constructs a `WatchRequest`, derives a resume key, and spawns a supervisor task.
2. The supervisor reads the cursor from the state store (if one is configured).
3. The supervisor sends a `POST /api/v1/watch` to the server with the filter and the cursor.
4. The server replies with an SSE stream. The supervisor reads chunks, feeds them into the `finesse` parser, and turns parsed frames into `Notification` values.
5. For each notification, the supervisor:
   - runs every configured trigger,
   - persists the previous notification's sequence to the state store (next-send commit),
   - sends the current notification on a bounded channel to the consumer.
6. On a `connection-closing` event, a transport error, a heartbeat timeout, or a 5xx, the supervisor reconnects using the right backoff for the cause.
7. On a terminal error (4xx other than 401, second 401 after refresh, required trigger failure exhausted retries, history gap), the supervisor closes the stream with the error and exits.

Dropping the `NotificationStream` cancels the supervisor cooperatively through a oneshot channel; the supervisor exits within one event-loop tick.

## Cancellation and shutdown

aviso is cooperative everywhere. There are three cancellation paths:

- **Per-stream**: dropping the `NotificationStream` drops a oneshot sender; the supervisor's `select!` notices and exits.
- **Parent-cascade**: dropping the last `AvisoClient` clone trips a `tokio::sync::watch` flip that every child supervisor observes.
- **Ctrl+C in the CLI**: a signal handler triggers a graceful drain via the same per-stream mechanism for every active listener.

The supervisor's `select!` is `biased` so cancellation cannot be starved by a fast stream. Every supervisor await (auth header, HTTP send, chunk read, channel send) is wrapped in the cancel arm.

## Why a single supervisor per watch

A bounded channel (capacity 128 by default; 1 when a state store is configured) sits between the supervisor and the consumer. The channel applies TCP backpressure end to end: when the consumer falls behind, the supervisor's `send` blocks, which makes it stop reading bytes from the wire, which throttles the server.

When a state store is wired, the channel capacity drops to 1 so the supervisor's commit-of-previous-notification is forced to happen before the consumer can pull the next one. The user-facing contract is "pulling item N+1 implies item N is durable", and capacity 1 is what makes that true.

## The state-store contract

The store is strictly monotonic: a `put` whose sequence is at or below the existing on-disk value is silently a no-op. This is what guarantees the cursor never moves backwards, even across concurrent writers or after operator errors editing the file.

For multiple processes, the file store uses an advisory `flock` on a sidecar lockfile. The lockfile is separate from the data file because the atomic-rename pattern would otherwise invalidate the lock between operations.

A failed `put` leaves both memory and disk unchanged. Aside: `put` and `delete` on the file store are not cancellation-safe; drive them to completion. The CLI does this.

## What is intentionally not here

- **Client-side schema validation**. The server is the single source of truth. aviso does not depend on `aviso-validators` and does not pre-validate notifications.
- **Auto-retry on `notify`**. A `POST` that fails after the request body has been sent might have been processed; a blind retry would risk a duplicate. When a server-side idempotency-key contract becomes available the policy will be revisited.
- **A metrics surface**. The core emits structured `tracing` events. Consumers that need Prometheus or OpenTelemetry metrics build a thin adapter on top.

## Where to go next

- [Library guide](./lib-guide.md): how to use the library from your own code.
- [Contributing](./contributing.md): tests, gates, the workflow.
