<!--
SPDX-FileCopyrightText: 2026 European Centre for Medium-Range Weather Forecasts (ECMWF)
SPDX-License-Identifier: Apache-2.0
-->

# Glossary

<div class="reference-guide">

Use this page to look up words in the guides. The links explain each idea in
more detail.

## Authentication provider

Supplies credentials, such as a token, to the client. This is different from a
data provider who publishes notifications. See
[Authentication providers](./auth-providers.md).

## Checkpoint and cursor

A cursor is the sequence position used to resume listening. A checkpoint saves
that position. It does not confirm that your analysis finished processing the
notification. See [Resume and state](./resume-and-state.md).

## Data provider

A person or service that publishes notifications about available data. Receiving
notifications does not require permission to publish them. See
[Publishing](../python/publish.md).

## Event type

A named kind of event, such as `mars`. Each type has a schema describing its
identifier fields. See [Notifications](./notifications.md).

## Filter

Conditions selecting which notifications you receive. Every condition must
match. See [Filters](./filters.md).

## Identifier

A named value describing the event, such as a forecast's class or step. The
schema defines the available fields. See [Notifications](./notifications.md).

## Listener

Receives matching notifications for an event type and filter. In the CLI, you
can give a listener a name and attach triggers in a YAML file, or specify its
event and filter on the command line. See
[Publish and listen](../cli/publish-and-listen.md).

## Notification

A message telling you something happened. The server sends it as a CloudEvent,
a JSON message with a standard set of fields. The client also exposes convenient
fields for the event type, sequence, identifiers and payload. See
[Notifications](./notifications.md).

## Payload

Extra information supplied by the data provider, such as a file location. It
may be `null`. Filters select identifiers, not payload fields. See
[Notifications](./notifications.md#payloads).

## Replay and retained history

Replay reads earlier notifications still stored by the server. Retention is how
long the server keeps them. Deleted or expired history cannot be replayed. See
[Replay history](../cli/replay.md).

## Resume

Continue listening after a recorded sequence position. Recovery needs retained
history and a usable position. Notifications may repeat, and saved progress does
not guarantee completed application work. See
[Resume and state](./resume-and-state.md).

## Schema

The server's rules for identifier names, their values and which fields a filter
must include. Data providers supply every declared identifier when publishing.
Inspect the rules with `aviso schema get <TYPE>`. See
[Schemas](../cli/operations.md#schemas).

## Sequence number

A 64-bit whole number assigned within an event type. Stored sequences increase,
but deliveries can repeat or arrive out of order. See
[Notifications](./notifications.md#sequence-numbers-and-ordering).

## State file

A local file of saved resume positions. The CLI defaults to
`~/.config/aviso/state.json`; Python has no state store unless you configure
one.
See [Resume and state](./resume-and-state.md).

## Stream

The connection over which a listener receives notifications. It uses
Server-Sent Events (SSE), which lets the server send messages as they become
available. See [Streams](./streams.md).

## Trigger

An action aviso runs for a matching notification, such as writing to a log or
calling a webhook. A webhook sends an HTTP request to another service. See
[Triggers overview](../triggers/overview.md).

## WatchRequest

The Rust request type describing what a listener should receive. Python's
`listen` method exposes the main choices as arguments. See the
[Rust API](../reference/rust-api.md) and
[Python listening guide](../python/listen.md).

</div>
