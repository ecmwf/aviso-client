# Resume and state

<div class="reference-guide">

A cursor records how far a listener has reached, using a notification's sequence
number. Saving it lets a later run request notifications after that position.
The server must still have the history you need.

The CLI saves state in `~/.config/aviso/state.json` by default. Python clients
have no state store by default. To save Python progress across runs, configure a
[file store](../python/state-and-resume.md).

A saved cursor does not confirm that your analysis finished. Notifications can
still be waiting in the client's queue when it saves progress. A crash can
therefore leave unfinished work behind the saved position. Track completed work
separately when you need to recover it, and make repeated processing safe.

## When the cursor advances

For each notification, the background listener:

1. Saves the previous pending sequence, if there is one and a store is set.
2. Runs this notification's triggers. Required triggers must succeed.
3. Places the notification in the queue for your code to read.
4. Marks its sequence as pending if it advances the current position.

These steps do not wait for your code to finish its work. If a required trigger
fails, this notification does not become pending, but the previous position
may already have been saved. The final pending position is saved on exit only
if `flush_cursor_on_exit` is enabled. Python defaults to `False`.

Saved positions only move forwards. Older notifications can still be delivered
without moving the saved position backwards.

## What at-least-once means for your triggers

A trigger can run more than once if the process stops after the action but
before its sequence is saved. This repeat behaviour is often called
at-least-once delivery. It is not a guarantee that every notification reaches
your application after a crash: recovery also needs a usable starting position
and retained history.

Where possible, design an action so repeating it has the same result as doing
it once. This is called being idempotent.

| Same result when repeated | Additional effect when repeated |
|---|---|
| Replace a file with the same contents. | Append another log entry. |
| Update a record using its unique id. | Insert another database row. |
| Record that an email was already sent. | Send another email. |

Use the pair `event_type@sequence` to recognise notifications you have already
handled. Recording this and performing the action must be coordinated if a
crash between the two would cause a problem.

## Optional triggers are different

A trigger with `required: false` in YAML is still attempted. If it fails, aviso
logs a warning and allows progress to continue. Its failure does not by itself
cause redelivery.

Use optional triggers for actions whose failure should not stop listening.
Use required triggers when a failed action should stop that listener.

## Where the file lives

For the CLI, the default paths are:

```text
~/.config/aviso/state.json
~/.config/aviso/state.json.lock
```

The lockfile coordinates access from cooperating processes. The JSON state
file is written when progress is saved; merely starting a listener does not
mean it has saved a position.

Change the location:

```yaml
# ~/.config/aviso/config.yaml
state_file: "/var/lib/aviso/state.json"
```

The equivalent CLI option is `--state-file`, followed by the local file path.

The CLI creates the parent directory for its configured state file. Library
callers using `JsonFileStore::open` directly must create the parent directory
first.

The state file must be on a local filesystem. NFS and CIFS are not supported
because the cross-process advisory lock does not work reliably on them.

## Running aviso without a state file

For one-off exploration where you do not want to save progress, add
`--no-state-store`. This example assumes the `mars` schema accepts `class: od`
and requires no other filter fields; check with `aviso schema get mars` first:

```bash
aviso listen --no-state-store --event mars --identifiers '{"class":"od"}'
```

aviso uses an in-memory store for the lifetime of the command. When the process
exits, the cursor is lost; the next run starts from "now" (or from your
`--from`, if you pass one).

## Replay does not touch the state file

`aviso replay` is stateless by design. It never reads or writes the state file.
The reason: replay would compute the same hash key as the equivalent listen, and
letting it advance the cursor could move the listen past notifications listen
has not processed.

The trade-off: if you interrupt a replay, the next replay needs an explicit
`--from`.

This rule describes the CLI command. Python replay-only listening still uses
the client's configured state store, if any; it is not automatically stateless.
For an independent inspection, use a Python client without a state store. See
[Python replay-only listening](../python/listen.md#replay-only) for the call.

## The state file format, very briefly

This shortened example shows the saved fields. A real key has 64 hexadecimal
characters; `46fe3e30...` stands for the full key here.

```json
{
  "version": 1,
  "key_format_version": 1,
  "checkpoints": {
    "46fe3e30...": {
      "last_committed_sequence": 72,
      "last_event_id": "mars@72"
    }
  }
}
```

- `version` and `key_format_version` are integer format versions.
- `checkpoints` contains saved positions. Each key is calculated from the server
  URL, event type and filter. Different filters can have separate
  positions.
- `last_committed_sequence` is the cursor. `last_event_id` is a human-readable
  form for the file's reader; aviso does not consult it on restart.

See the [state file reference](../reference/state-file.md) for every field and
how to change saved state safely.

## Rewinding the cursor

A rewind reads from an earlier position. A sequence start is exclusive:
`--from 41` in the CLI or `start_from=41` in Python reads strictly after
sequence 41, not including 41. You receive only matching notifications that the
server still stores.

Choose whether you want a one-time read or a new saved position:

1. For a one-time read, use CLI replay or a Python client without a state store.
   You can also give a listener an earlier explicit start. It may repeat
   notifications, but its saved position will not move backwards.
2. To replace saved positions, stop aviso before removing its state file. Start
   again with your chosen starting position. Once progress has been saved,
   remove the explicit start so later runs use the saved position.
3. Do not edit a running listener's state file to move its position backwards.

Deleting a state file removes every saved position in that file, not just the
listener you are investigating. Prefer replay for a one-time inspection.

An explicit start takes precedence over saved state. If you leave it in a
recurring command or script, every restart requests that same starting point.

## What next

- [State file reference](../reference/state-file.md): annotated example, edit
  safety, recovery from a format mismatch.
- [Streams](./streams.md): when the cursor matters (every reconnect).
- [Filters](./filters.md): how the hash key is derived (filter content matters).

</div>
