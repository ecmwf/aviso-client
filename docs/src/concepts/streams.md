<!--
SPDX-FileCopyrightText: 2026 European Centre for Medium-Range Weather Forecasts (ECMWF)
SPDX-License-Identifier: Apache-2.0
-->

# Streams and reconnects

<div class="reference-guide">

When you run `aviso listen`, aviso opens a long-lived HTTP connection to the
server and receives notifications as they arrive. This connection is called a
stream. The server sends messages over it using Server-Sent Events (SSE).

The listener first reads any requested history that the server still stores,
then waits for new matches. Without a starting position or saved state, it
waits for new notifications. Replay-only requests stop after stored history.

aviso reconnects automatically after routine interruptions. Backoff means
waiting before another attempt, so it does not repeatedly contact a busy
server. The details below help explain pauses and errors.

## Confirming a stream

The client accepts only HTTP 200 with the `text/event-stream` media type. Media
type matching ignores case and accepts parameters such as `charset=utf-8`.
It then waits for an Aviso opening control: `connection_established` on a
live-only watch, or `replay_started` when reading history. A saved resume cursor
also makes the request historical. Every reconnect validates its own opening.

Headers and confirmation share a ten-second deadline per connection. Heartbeats
and unknown SSE events cannot extend that deadline or mark the listener ready.
For non-200 responses, a diagnostic body that exceeds the deadline is omitted;
the HTTP status still determines whether to retry or stop.
Retry backoff resets only after confirmation. Once confirmed, the normal
heartbeat watchdog applies; time spent handling notifications or waiting for a
slow consumer does not count as network silence.

The CLI also applies an initial 30-second budget across retries, configurable
with `aviso listen --startup-timeout`. Library consumers have no initial retry
budget unless they explicitly set one. A healthy idle stream outlives either
startup deadline.

## The connection lives a while, then closes

aviso-server intentionally closes each watch connection after a configured
maximum duration (by default, one hour). The server signals this politely with a
`connection-closing` event whose reason is `max_duration_reached`.

aviso does not treat this as an error. It reconnects immediately, with no
backoff, and resumes from the next sequence. From your point of view, the
listener just keeps running.

## When aviso reconnects, and what kind of backoff

| What happened | What aviso does |
|---|---|
| Routine connection time limit | Reconnect immediately. |
| Server shutdown | Wait, then reconnect. |
| Connection drops | Retry with increasing delays. |
| Busy server (429 or 503) | Use the server's retry delay. |
| Rejected credentials (401) | Refresh credentials and retry once. |
| Other client error (4xx) | Stop the listener. |

For a dropped connection, the delay limit starts at 250 milliseconds and doubles
up to 30 seconds. The actual delay is random within that limit so listeners do
not all reconnect together. A server's `Retry-After` delay is capped at five
minutes. A second 401 in the same attempt cycle stops the listener.

## Heartbeats

The server sends a small heartbeat event periodically (every 30 seconds by
default). If aviso does not see any event (heartbeat, notification, or control)
for longer than `max(3 × heartbeat_interval, heartbeat_interval + 30 s)`, it
declares the connection silently dead and reconnects.

This check can detect a stalled network connection even when no explicit error
arrives, for example after a laptop wakes up on a different WiFi network.

## Reconnect-as-normal

You normally leave the same listener running through routine reconnects. The
background component managing the connection, called the supervisor, retries
recoverable failures and reports errors it cannot recover from.

## Resume after a reconnect

After progress has been recorded, reconnecting requests notifications after
that sequence. The running listener keeps this position in memory even without
a state file. Saved state lets a later run use it too.

Recovery depends on history still being available on the server. A saved
position does not prove your application finished processing every notification
or guarantee that every queued notification reaches your code after a crash.

For the full rules of how cursors advance, see
[Resume and state](./resume-and-state.md).

## When the supervisor gives up

Some errors are terminal. The supervisor closes the stream and stops the
listener:

- A 4xx other than 401 or 429 from the server (the request is rejected; retrying
  will not help).
- A second 401 in the same cycle after a refresh attempt (the credential really
  is rejected).
- A required trigger that fails after all retries (the work cannot complete).
- A history gap that the server has declared (the notification you need has been
  pruned).
- A malformed event from the server (a CloudEvents id that does not parse, or
  a notification for an event type other than the one the listener asked for).

The CLI reports the failed listener's error. Other listeners in the same
`aviso listen` command keep running. After all listeners stop, the command exits
with code 1 if any listener failed.

## What next

- [Resume and state](./resume-and-state.md): how aviso remembers where it was.
- [Filters](./filters.md): what the server uses to decide which events to send
  you.
- [Troubleshooting](../cli/troubleshooting.md): the common failure modes.

</div>
