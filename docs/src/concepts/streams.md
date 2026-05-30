# Streams and reconnects

When you run `aviso listen`, aviso opens a long-lived HTTP connection to the
server (server-sent events) and reads notifications off it as they arrive.

You will see this work just fine on its own: aviso reconnects when needed,
applies backoff, and resumes where it left off. The details below are useful
when you are troubleshooting or building a long-running service.

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
| `connection-closing` with `max_duration_reached` | Reconnect immediately. The normal case. |
| `connection-closing` with `server_shutdown` | Wait a few seconds, then reconnect. |
| TCP connection drops without a polite close | Reconnect with exponential backoff (250 ms doubling, capped at 30 s). |
| Server returns `429 Too Many Requests` or `503 Service Unavailable` | Honour the `Retry-After` header, then reconnect. Capped at five minutes. |
| Server returns `401 Unauthorized` | Ask the auth provider to refresh, then retry. A second 401 in the same cycle surfaces as an error. |
| Server returns any other 4xx (`403`, `404`, `410`) | Stop. This is a permanent error. |

The backoff uses full jitter so a fleet of listeners does not all reconnect in
lockstep after a server outage.

## Heartbeats

The server sends a small heartbeat event periodically (every 30 seconds by
default). If aviso does not see any event (heartbeat, notification, or control)
for longer than `max(3 × heartbeat_interval, heartbeat_interval + 30 s)`, it
declares the connection silently dead and reconnects.

The watchdog catches problems that TCP keepalive does not, such as:

- A NAT or firewall that has silently dropped your connection.
- A reverse proxy whose upstream has hung but whose TCP socket is still alive.
- A laptop that just woke up from sleep on a new WiFi network.

## Reconnect-as-normal

The cumulative effect is that a single `aviso listen` command can run for weeks
across an arbitrary number of routine server-driven reconnects without ever
surfacing a transient error to you. Failure surfaces only when the supervisor
cannot recover.

## Resume after a reconnect

Every reconnect uses the cursor in the state file (or what aviso has in memory
if you are running with `--no-state-store`). The supervisor asks the server for
"everything from sequence `last_committed + 1` onwards", so no notification is
lost.

For the full rules of how cursors advance, see
[Resume and state](./resume-and-state.md).

## When the supervisor gives up

Some errors are terminal. The supervisor closes the stream and stops the
listener:

- A 4xx other than 401 from the server (the request itself is wrong; retrying
  will not help).
- A second 401 in the same cycle after a refresh attempt (the credential really
  is rejected).
- A required trigger that fails after all retries (the work cannot complete).
- A history gap that the server has declared (the notification you need has been
  pruned).
- A malformed event from the server (a CloudEvents id that does not parse).

In each case the CLI exits with code 1 and a final error line. Other listeners
in the same `aviso listen` invocation keep running.

## What next

- [Resume and state](./resume-and-state.md): how aviso remembers where it was.
- [Filters](./filters.md): what the server uses to decide which events to send
  you.
- [Troubleshooting](../cli/troubleshooting.md): the common failure modes.
