# Key concepts

Concise primer on the moving parts of aviso, written from the client's point of view.

## Notifications, streams, schemas

`aviso-server` exposes named **streams** (event types). Each stream has a **schema** describing which **identifier fields** are valid and whether a JSON **payload** is required. A **notification** is one event published into a stream. Clients can:

- **publish** notifications (`POST /api/v1/notification`);
- **watch** a stream live (`POST /api/v1/watch` with Server-Sent Events response);
- **replay** historical notifications (`POST /api/v1/replay`, also SSE).

## Filters

A watch/replay request carries an `identifier` map. The server returns only the notifications whose identifier matches every field. Spatial filters (`polygon`, `point`) are supported for streams whose schema includes geometry.

## SSE, but not Last-Event-ID

`aviso-server` streams notifications as SSE but does **not** emit the SSE `id:` line. Instead, every CloudEvent payload carries an `id` field of the form `<event_type>@<sequence>`, and the client extracts the integer sequence from there. To resume, the client re-issues the watch/replay request with `from_id = last_committed_sequence + 1` (sequence-based) or `from_date = ...` (timestamp-based). See [Resume & state](../resume/overview.md).

## Connection lifetime

The server deliberately closes watch connections after a configured maximum duration (default one hour), signalled by a `connection-closing` SSE event with `reason: "max_duration_reached"`. **Reconnects are normal**, not an error condition. The client treats them as routine and resumes from its last committed sequence.

## At-least-once delivery

The client guarantees at-least-once delivery: every notification it successfully processes (and where applicable, hands to a trigger) is checkpointed; after a crash or reconnect, the client resumes from the last *committed* sequence. Triggers may therefore see the same notification more than once and should be designed to be idempotent. See [Resume & state](../resume/overview.md).
