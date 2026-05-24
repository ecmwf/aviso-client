# State file reference

The on-disk record of where each listener left off. By default at `~/.config/aviso/state.json`. A sibling lockfile lives next to it at `~/.config/aviso/state.json.lock`.

If you have never run `aviso listen`, neither file exists yet. Both are created lazily on the first successful checkpoint commit. `aviso replay` does not touch either file.

For the conceptual model, read [Resume and state](../concepts/resume-and-state.md) first.

## A worked example

After listening for `mars` events with `class=od` and processing one notification, the file looks like:

```json
{
  "version": 1,
  "key_format_version": 1,
  "checkpoints": {
    "46fe3e30937478ae30527c74cb2f03a5e5419228a11051cdaccefe7d044b21c3": {
      "last_committed_sequence": 72,
      "last_event_id": "mars@72"
    }
  }
}
```

The next time aviso runs against the same server, event type, and filter, it reads this checkpoint and asks the server for sequence 73 onwards.

## What each field is for

### `version`

The format version of the JSON document itself (which top-level fields exist, what types they hold).

The check is **exact match**. A file with `version: 2` is rejected by a client that knows only `version: 1`. If you upgrade aviso and the version bumps, the old file becomes unreadable until you either install a matching aviso, or delete the file and accept that the next run starts from "now".

### `key_format_version`

The version of the recipe used to derive the hex keys in `checkpoints`. Also exact match. A mismatch means the keys on disk were hashed differently and cannot be matched to anything the live client computes.

### `checkpoints`

A map keyed by 64-character hex resume keys. Each value is one checkpoint object.

| Subfield | Type | Meaning |
|---|---|---|
| `last_committed_sequence` | unsigned 64-bit integer | The cursor. The sequence of the last fully-processed notification. |
| `last_event_id` | string or `null` | The notification's id in the `<event_type>@<sequence>` form, for humans reading the file. aviso does not consult it on restart. |

## What "committed" actually means

`last_committed_sequence` advances only after every required trigger for the notification succeeded *and* aviso wrote the new value to disk. If anything in that chain fails, the cursor does not advance, and the notification will be redelivered on the next run.

So when you see `last_committed_sequence: 72`, that means `mars@72` was fully processed end-to-end. The cursor is a high-water mark of completed work, not a "received" counter.

## Can I edit or delete this file?

### Delete the whole file: safe, with consequences

```bash
rm ~/.config/aviso/state.json ~/.config/aviso/state.json.lock
```

Do this only when no `aviso listen` is running. The next run starts with no cursor:

- If your YAML or command-line sets `--from`, the run resumes from there.
- Otherwise the run starts from "now" (the server's current tip).

You will miss the notifications between the deleted cursor and "now" unless you explicitly replay them.

### Delete just the lockfile: unsafe while a client is running

```bash
rm ~/.config/aviso/state.json.lock
```

The cross-process lock is attached to the lockfile's inode. Deleting the lockfile while a client holds the lock lets a sibling process acquire "the" lock on a fresh inode, which breaks mutual exclusion. Only delete it when no aviso process is using the file.

### Hand-edit the file: mostly read-only by design

The store guards `last_committed_sequence` against going backwards. Two consequences:

- **Lowering a sequence is silently ignored.** If you edit `72` down to `42` and start aviso, the next checkpoint write compares the on-disk value against the in-memory candidate and keeps the higher. Your edit is overwritten on the next commit.
- **Raising a sequence skips notifications.** If you edit `72` up to `100`, the next reconnect asks the server for sequence 101 onwards, and notifications 73 through 100 are never delivered to this listener. aviso has no way to tell whether that was intentional, so it trusts you.

To genuinely reset a cursor, delete the file (with aviso stopped) and start with `--from`:

```bash
rm ~/.config/aviso/state.json
aviso listen my-listeners.yaml --from 2026-05-23T00:00:00Z
```

### `cat` while aviso is running: safe

Reading the file from outside aviso is harmless. The atomic-write protocol guarantees you see either the previous complete file or the new complete file, never a half-written tear.

## How `--from` interacts with the state file

When you pass `--from <VALUE>` and a cursor already exists in the file:

- aviso uses your `--from` for the initial seek.
- As the rewind delivers notifications, the state file ignores updates whose sequence is at or below the existing high-water mark. Your `--from` cannot push the cursor backwards.
- Once the run advances past the previous high-water mark, normal updates resume.
- Restarting without `--from` honours the stored cursor again.

Use `--from` as a one-shot rewind. Leaving it in a systemd unit means redelivery from that point on every restart.

## Recovery from a format-version mismatch

```text
state file format version 2 does not match supported version 1
```

or:

```text
state file uses key_format_version 2; this client uses 1
```

Three options:

1. **Match the client to the file.** Install an aviso whose `version` and `key_format_version` match what is on disk. This preserves cursors.
2. **Migrate the file manually.** Only viable if you know the layout differences between the two versions. There is no automated tool.
3. **Accept the loss.** Stop any running clients, delete both files, restart. Cursors are lost; the next run begins from "now" or from `--from`.

Pin the aviso binary version next to your state file in production so the mismatch only surfaces during a controlled upgrade.

## Where the file must live

A local filesystem. The cross-process advisory lock uses POSIX semantics that are not reliable on NFS or CIFS.

When using `JsonFileStore` directly, the parent directory must already exist. The CLI creates the parent directory for its configured state file before opening the store.

## What is in the hex key

The 64-character hex keys are the SHA-256 of a deterministic, length-prefixed byte sequence built from:

1. The format version.
2. The normalised server base URL (lowercase scheme and host, default ports stripped, userinfo removed).
3. The event type.
4. The filter, canonicalised so `{"a":1,"b":2}` and `{"b":2,"a":1}` hash the same way.
5. An optional schema fingerprint.

Two listeners with different filters get different keys. Two listeners on different servers also get different keys.

The hex is a hash, not a literal. Filter contents do not appear on disk in plain text.

## What next

- [Resume and state](../concepts/resume-and-state.md): the concept.
- [CLI configuration](../cli/configuration.md): how to change where the file lives.
- [Troubleshooting](../cli/troubleshooting.md): when things go wrong.
