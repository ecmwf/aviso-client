# Concepts in five minutes

The five ideas you need to get fluent with aviso. Each links to a deeper page if
you want more.

## Notifications

A notification is one event from the server. It has an event type, a set of
identifiers, an optional JSON payload, and a sequence number that strictly
increases.

You ask the server "let me know when something of type `mars` arrives matching
these identifiers", and the server streams the matching events to you.

Read [Notifications](../concepts/notifications.md) for the wire format and the
fields.

## Streams

When you listen, aviso opens a long-lived HTTP connection (an SSE stream) to the
server. The server pushes events to you as they happen. The connection
deliberately closes after a configured maximum duration; aviso reconnects
automatically. Reconnects are normal, not an error.

Read [Streams](../concepts/streams.md) for the reconnect rules and the heartbeat
watchdog.

## Filters

A filter is the set of identifiers you want notifications for.
`{"class":"od","stream":"oper"}` says "only events where the identifier map
matches both of these". The server enforces filters; aviso just passes them
along.

Some identifier fields are required by the server's schema, others are optional.
Omitted optional fields act as wildcards.

Read [Filters](../concepts/filters.md) for the matching rules and the spatial
filter shapes.

## Resume and state

When aviso processes a notification end-to-end (including running every required
trigger), it writes the sequence number to a small JSON file. On restart, aviso
reads that file and resumes from the next sequence. You will not miss events and
you will not skip ahead.

The file lives at `~/.config/aviso/state.json` by default. You can change it,
disable it for a one-off run, or delete it when you want to start fresh.

Read [Resume and state](../concepts/resume-and-state.md) for the durability
rules and the rewind workflow.

## Triggers

A trigger is what aviso does with each notification. Six built-in kinds are
available:

- `echo` prints the notification.
- `log` appends it to a file.
- `command` runs a shell command (Unix only).
- `webhook` makes an HTTP request to a URL of your choice.
- `teams` posts to a Microsoft Teams channel.
- `post` forwards the original event envelope to another service.

You can attach as many triggers as you want to a listener. They run in order,
and the sequence number only advances when every required trigger succeeds.

Read the [Triggers overview](../triggers/overview.md) for the dispatcher
contract and pick a kind to dive into.

## Where to go next

- [Quickstart](./quickstart.md) if you want to try it now.
- [CLI overview](../cli/overview.md) if you are setting up a listener.
- [Library guide](../developers/lib-guide.md) if you are embedding aviso in your
  own Rust code.
