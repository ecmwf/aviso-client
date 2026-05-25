# Glossary

The short version of the words aviso uses. Each links to the page where the idea lives.

**Cursor.** The sequence number aviso uses on the next reconnect to ask the server "what happens after this one?". The cursor advances after each notification finishes processing. See [Resume and state](./resume-and-state.md).

**Event type.** A named kind of event the server publishes. For example `mars`. The server has one schema per event type. See [Notifications](./notifications.md).

**Filter.** The map of identifiers you want notifications for. The server returns only events whose identifier matches every field you set. See [Filters](./filters.md).

**Identifier.** A key-value pair that describes which event a notification is. The schema declares which identifier keys exist for an event type. See [Notifications](./notifications.md).

**Listener.** A subscription with a name, an event type, a filter, and a set of triggers. Defined in a YAML file or constructed inline with `--event` and `--identifiers`. See [CLI publish and listen](../cli/publish-and-listen.md).

**Notification.** One event delivered to you. It carries an event type, a sequence number, an identifier map, and an optional payload. See [Notifications](./notifications.md).

**Resume.** Picking up after a restart from the last fully processed notification, so normal restarts avoid skipping events. At-least-once delivery can still redeliver a notification. See [Resume and state](./resume-and-state.md).

**Schema.** The server's declaration of which identifier fields exist for an event type, which are required, and what types they hold. Inspect with `aviso schema get <TYPE>`. See [CLI operations](../cli/operations.md#schemas).

**Sequence number.** A 64-bit integer that strictly increases per event type. The cursor lives in this space. See [Notifications](./notifications.md).

**State file.** The on-disk record of cursors per listener. By default `~/.config/aviso/state.json`. See [State file](../reference/state-file.md).

**Stream.** The long-lived HTTP connection aviso opens to receive notifications. The transport is Server-Sent Events. See [Streams](./streams.md).

**Trigger.** The action aviso takes for each matching notification: echo, log, command, webhook, teams, post. Configured in the listener YAML. See [Triggers overview](../triggers/overview.md).
