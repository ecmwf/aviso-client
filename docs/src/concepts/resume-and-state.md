# Resume and state

aviso processes every notification end to end before it commits the sequence
number. If aviso restarts, the next run starts from the last committed sequence.
This avoids skipping ahead; at-least-once delivery means the same notification
can still run more than once.

The cursor lives in a small JSON file on disk. By default,
`~/.config/aviso/state.json`.

## When the cursor advances

Each notification goes through these steps:

1. The server sends it on the SSE stream.
2. aviso decodes it.
3. **Every required trigger runs successfully** (echo, log, command, webhook,
   teams, post).
4. aviso writes the new sequence to disk and asks the kernel to flush.

Only after step 4 does the cursor advance. If step 3 fails for a required
trigger, the cursor stays where it was. On the next run, aviso re-delivers the
same notification and runs the triggers again.

This is the **at-least-once delivery** rule. It is the most important thing to
know about how aviso behaves.

## What at-least-once means for your triggers

A required trigger might run more than once for the same notification (after a
crash, after a network blip during the commit step). Triggers should produce the
same observable outcome either way.

| Idempotent (safe) | Not idempotent |
|---|---|
| Appending the notification to a log file. | Sending an email. Each run sends another email. |
| `kubectl annotate ... --overwrite`. | `psql -c "INSERT ..."` without `ON CONFLICT`. |
| `cp src dest` (copy is overwriting). | `kubectl create ...` (fails on conflict). |

For triggers that are not inherently idempotent, build a dedupe layer keyed on
`event_type@sequence`.

## Optional triggers are different

A trigger with `required: false` (in YAML) is fire-and-forget. Its failure logs
a `WARN` and the cursor still advances. Optional triggers do not cause
redelivery.

Use optional for "nice to have" sinks (a metrics endpoint, a backup webhook).
Use required for the work you cannot lose.

## The state file format, very briefly

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
- `checkpoints` is a map. The hex key is a hash of the server URL, the event
  type, and the filter. Two listeners with different filters get different
  cursors.
- `last_committed_sequence` is the cursor. `last_event_id` is a human-readable
  form for the file's reader; aviso does not consult it on restart.

The full reference (every field, edit safety, how to genuinely rewind) is at
[State file](../reference/state-file.md).

## Where the file lives

By default:

```text
~/.config/aviso/state.json
~/.config/aviso/state.json.lock
```

Both are created lazily on the first commit. If you have never run
`aviso listen`, neither file exists.

Change the location:

```yaml
# ~/.config/aviso/config.yaml
state_file: "/var/lib/aviso/state.json"
```

or:

```bash
aviso listen --state-file /var/lib/aviso/state.json ...
```

The CLI creates the parent directory for its configured state file. Library
callers using `JsonFileStore::open` directly must create the parent directory
first.

The state file must be on a local filesystem. NFS and CIFS are not supported
because the cross-process advisory lock does not work reliably on them.

## Running aviso without a state file

For one-off exploration where you do not want to persist anything:

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

## Rewinding the cursor

A "rewind" pushes the cursor back to an earlier point. Three approaches,
depending on how committed you are:

1. **One-shot rewind**: run `aviso listen ... --from <earlier>` once and stop.
   The state file's high-water mark is preserved (a guard in the file store
   ignores updates that would move the cursor backwards). You will redeliver the
   notifications between `<earlier>` and the previous cursor, but the file does
   not regress.
2. **Permanent rewind**: stop aviso, delete the state file, restart with
   `--from <earlier>` once, then remove `--from` from the invocation. The new
   cursor takes hold from the first commit.
3. **Surgical, no downtime**: not supported. The store guards against backwards
   movement on purpose.

A long-lived systemd unit that keeps `--from <date>` in its `ExecStart` will
redeliver from that date on every restart, forever. `--from` is meant as a
one-shot operator decision, not a permanent setting.

## What next

- [State file reference](../reference/state-file.md): annotated example, edit
  safety, recovery from a format mismatch.
- [Streams](./streams.md): when the cursor matters (every reconnect).
- [Filters](./filters.md): how the hash key is derived (filter content matters).
