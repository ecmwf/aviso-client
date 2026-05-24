# The state file

This page is for operators who have just spotted `~/.config/aviso/state.json` on disk and want to know what it is, whether they can touch it, and what every field in it means. If you only want the concepts ("what is a checkpoint?", "how does resume work?"), read [Resume and state](./overview.md) first.

## Where the file lives

By default the `aviso` CLI keeps its resume cursors in:

```text
~/.config/aviso/state.json
```

A sibling lockfile sits next to it:

```text
~/.config/aviso/state.json.lock
```

Both are created lazily on the first successful checkpoint commit. If you have never run `aviso listen`, neither file will exist yet, and that is correct. `aviso replay` does not touch the state file at all; see [`aviso replay` and the state file](#aviso-replay-and-the-state-file) below.

### Changing the location

Set `state_file:` in your CLI config file:

```yaml
# ~/.config/aviso/config.yaml
state_file: "/var/lib/aviso/state.json"
```

or pass `--state-file <path>` on the command line (the flag overrides the config). The path must be on a **local filesystem**; NFS and CIFS are not supported because the cross-process advisory lock relies on POSIX `flock` semantics that those filesystems do not provide. The parent directory must already exist; the file itself is created on the first checkpoint.

### Disabling persistence entirely

Pass `--no-state-store` to `aviso listen` to drop straight into an in-memory store for the lifetime of the command. The cursor is lost when the process exits, so the next run starts from the server's `from_id`/`from_date` you specified (or from "now" if you specified neither).

The flag is `aviso listen`-only. `aviso replay` does not use the state file at all and has no `--no-state-store` flag; see the next section.

## `aviso replay` and the state file

`aviso replay` deliberately does NOT touch the state file. A replay run never reads a checkpoint from it, never writes one to it, and never opens the lockfile. The `--state-file` / `AVISO_STATE_FILE` settings are silently ignored by `aviso replay`.

### Why?

A replay run shares the same `ResumeKey` derivation with `aviso listen` (`server URL + event type + filter`; see [Resume and state](./overview.md#the-key-resumekey)). If both commands wrote to the same state file against the same listener YAML, they would collide on the key, and the monotonic-merge rule (higher candidate wins) would let replay advance listen's cursor past events listen has not actually processed. That would break listen's at-least-once delivery guarantee.

Keeping replay stateless avoids the hazard entirely. The trade-off is that an interrupted replay (Ctrl+C, network drop) is not auto-resumable: the next `aviso replay` invocation needs an explicit `--from <VALUE>` pointing at the desired resume cursor, computed manually from the most recent delivered notification. A future release may add resumable replays via a separate state namespace; for now, replay is one-shot by design.

## A worked example

After running `aviso listen` against `aviso-server.ecmwf.int` with a single listener watching `mars` events filtered on `class=od`, processing one notification, then pressing Ctrl+C, the file looks like:

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

The next time `aviso listen` runs against the same server + event type + filter, it reads this checkpoint and asks the server for `from_id = 73` (the next sequence after `72`), so no notifications are dropped and none are redelivered.

## What each field is for

### `version`

The **file format version**. Tracks the shape of the JSON document itself (which top-level fields exist, what type each is, how `Checkpoint` is laid out).

The check is **exact match**, not "minimum supported". A file with `version: 2` is rejected by a client that knows `version: 1`, and vice versa, because either direction risks the client misinterpreting a field whose meaning has changed. The error surfaces as:

```text
state file format version 2 does not match supported version 1
```

If you upgrade your `aviso` binary and the version bumps, the old file becomes unreadable until you either:

1. Run a version of `aviso` that understands the version on disk, or
2. Delete the file (`rm ~/.config/aviso/state.json` plus the `.lock` while no client is running) and accept that the next run starts from "now" with no cursor.

### `key_format_version`

The **hash-input version** for the keys inside `checkpoints`. Tracks how the 64-character hex keys are derived.

Like `version`, this is exact match. A mismatch means the keys on disk were hashed with a different recipe and cannot be matched to anything the live client computes, so the file is unusable. The error message tells you the recovery options directly:

```text
state file uses key_format_version 2; this client uses 1
```

Recovery is the same as for `version`: run a client that matches, or delete and restart.

### `checkpoints`

A map keyed by **64-character hex resume keys** (see [The checkpoint key](#the-checkpoint-key) below). Each entry's value is a `Checkpoint` object:

| Field | Type | Meaning |
| --- | --- | --- |
| `last_committed_sequence` | unsigned 64-bit integer | The cursor. The sequence of the last fully-processed notification. |
| `last_event_id` | string or `null` | Diagnostic only; the human-readable `<event_type>@<sequence>` form. |

## What "committed" means

`last_committed_sequence` advances **only after the work succeeded**. Specifically:

1. The server sent a notification on the SSE stream.
2. The client decoded it successfully.
3. **Every required trigger ran successfully** (`echo`, `log`, `command`, `webhook`).
4. The file store wrote the new sequence to disk and `fsync`ed.

Only then does the next reconnect or restart resume from `last_committed_sequence + 1`. If step 3 fails for a required trigger, the sequence is **not** advanced, and the client will redeliver that notification on the next start. This is the at-least-once delivery guarantee.

So when you see `last_committed_sequence: 72`, that means `mars@72` was actually processed end-to-end, not merely received. The number is a high-water mark of completed work.

`last_event_id` is purely a convenience for humans reading the file. The reconnect logic never consults it. If it gets stale (because a future version stops emitting it), nothing breaks.

## `--from` interaction

When both a stored cursor AND a `--from <VALUE>` flag are present at process start, the rule is:

**`--from` always wins for the initial seek.** The state file's cursor is ignored at startup; the supervisor calls the server with the value you supplied. The CLI documents this alongside the seven accepted formats at [Command-line interface: `--from` formats](../usage/cli.md#from-value-formats).

The second-order effect is what most operators don't expect, and it is **protective**: the monotonic-merge rule in the file store means a rewind never regresses the on-disk cursor.

### Worked example

State file before:

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

Run `aviso listen --from 70 my-listeners.yaml`. The supervisor:

1. Asks the server for `from_id = 70`.
2. Receives `mars@71`, runs the listener's triggers, calls `store.put(key, Checkpoint::new(71, ...))`. The merge step compares candidate `71` against disk `72`: **disk wins; the put is silently a no-op.**
3. Receives `mars@73`, runs triggers, calls `store.put(key, Checkpoint::new(73, ...))`. Candidate `73` > disk `72`: **disk updates to 73.**
4. Receives `mars@74`, runs triggers, `put(74)` lands; disk is now `74`.
5. Operator hits Ctrl+C.

State file after:

```json
{
  "version": 1,
  "key_format_version": 1,
  "checkpoints": {
    "46fe3e30...": {
      "last_committed_sequence": 74,
      "last_event_id": "mars@74"
    }
  }
}
```

Sequence `71` was delivered to the triggers (at-least-once redelivery from the rewind), but the file's high-water mark stayed at or above `72` throughout. The next restart **without** `--from` resumes from `74 + 1 = 75`, not from `70` again.

### The systemd-unit footgun

Because precedence is re-evaluated at every process start, leaving `--from <date>` in a long-lived unit file means the service will redeliver from that date on every restart, *forever*. The state file's advancing cursor has no effect because `--from` overrides it again on each launch.

The right mental model is:

- **`--from` is a one-shot operator decision.** Use it to perform a manual rewind, then remove it from the invocation.
- **The state file is the persistent default.** It is what the supervisor consults when `--from` is absent.

If you need a long-lived service that always replays from a fixed point, that is unusual and probably a misunderstanding; you want a regular watch with a fresh state-store path per deployment instead.

### How to genuinely rewind a cursor

Three options, depending on how far back you need to go and how concurrent the writers are:

1. **One-shot rewind.** `aviso listen ... --from <earlier>`, let it run, Ctrl+C. The state file's high-water mark is preserved; redelivery happens for [`--from`, high-water mark] and the file advances naturally past the high-water mark from there.

2. **Permanent rewind.** Stop the client. `rm state.json state.json.lock`. Restart with `--from <earlier>` once. Remove `--from` from the invocation. The new cursor takes hold from the first commit.

3. **Surgical, no client downtime.** Not supported. The store enforces strict-monotonic `put` precisely so a misbehaving caller cannot regress the cursor; that protection also blocks operator surgery. Use option 1 or 2.

## "Can I edit or delete this file?"

### Delete the whole file: **safe** (with consequences)

```bash
rm ~/.config/aviso/state.json ~/.config/aviso/state.json.lock
```

Do this only when no `aviso listen` is running (replay does not hold the lock or open the file, so it is irrelevant to this rule). The next run starts with no cursor, so:

- If your YAML or command-line set `--from <sequence-or-date>`, the run resumes from there.
- Otherwise the run starts from "now" (the server's current tip).

You will not get the notifications between the deleted cursor and "now" unless you explicitly replay them.

### Delete just the lockfile: **unsafe while a client is running**

```bash
rm ~/.config/aviso/state.json.lock
```

The cross-process lock is attached to the lockfile's inode. Deleting the lockfile while a client holds the lock lets a sibling process acquire "the" lock on a fresh inode, breaking mutual exclusion and potentially corrupting concurrent writes. Only delete the lockfile when no `aviso` process is using the state file.

### Hand-edit the file: **mostly read-only by design**

The store enforces strict-monotonic `last_committed_sequence`. That has two practical consequences:

- **Lowering a sequence is silently ignored**. If you edit `last_committed_sequence: 72` down to `42` and start a client, the next checkpoint write merges the on-disk value against the in-memory candidate and keeps the higher of the two. Your edit is overwritten on the next commit.
- **Raising a sequence skips notifications**. If you edit `72` up to `100` and start a client, the reconnect asks the server for `from_id = 101`, and notifications 73 through 100 are never delivered to this client. The store has no way to know whether that was intentional, so it trusts you.

If you genuinely want to reset a cursor, the right workflow is:

```bash
# Client must be stopped first.
rm ~/.config/aviso/state.json
# Start the client with --from <sequence> or --from <date> to seed a fresh cursor.
aviso listen my-listeners.yaml --from 2026-05-23T00:00:00Z
```

Editing the hex key is even less useful: the next client run recomputes the key from the live config and ignores any orphaned entries on disk.

### A `cat` while a client is running: **safe**

Reading the file from outside the client is harmless. The atomic-write protocol guarantees a `cat` either sees the previous complete file or the new complete file, never a half-written tear.

## Recovery from a version mismatch

If `aviso listen` refuses to start with:

```text
state file format version 2 does not match supported version 1
```

or:

```text
state file uses key_format_version 2; this client uses 1
```

three paths forward:

1. **Match the client to the file.** Install (or revert to) a version of `aviso` whose `version` and `key_format_version` match what is on disk. This preserves all stored cursors.
2. **Migrate the file manually.** Possible only if you understand the on-disk layout differences between the two versions and can rewrite the file accordingly. There is no automated migration tool.
3. **Accept the loss of cursors.** Stop any running clients, `rm` both the state file and the lockfile, then restart. Cursors are lost; the next run begins from "now" or from the `--from` you supply.

Production deployments should pin the `aviso` binary version next to the state file so the mismatch only surfaces during a controlled upgrade.

---

# Internals

The sections above cover everything an operator needs. The rest of this page is for the curious; nothing here is required to use the file.

## The checkpoint key

The 64-character hex keys in `checkpoints` are the **lowercase hex SHA-256 digest** of a deterministic, length-prefix-framed byte sequence. The inputs, in order:

1. `key_format_version` as a little-endian 4-byte unsigned integer.
2. The normalised server base URL (length-prefixed).
3. The event type (length-prefixed).
4. The filter body, canonicalised via [RFC 8785 JSON Canonicalization Scheme](https://datatracker.ietf.org/doc/html/rfc8785) (length-prefixed).
5. Optional schema fingerprint (one tag byte plus length-prefixed bytes if present, one zero byte if absent).

The "length-prefixed" framing means every variable-length field is preceded by its byte length as a little-endian 8-byte unsigned integer. That guarantees no two distinct logical inputs can collide, regardless of what bytes (including NUL) appear inside any field.

### Why a hash and not a literal `mars:{class=od}` key?

Five reasons, design-driven:

1. **Fixed-size keys**. Regardless of filter complexity (a 50-vertex polygon, a deeply nested object), every key on disk is 64 hex characters.
2. **No escaping headaches**. A literal key would have to handle filters containing `:`, `/`, `{`, `}`, quotes, NUL bytes. The hash sidesteps all of that.
3. **Privacy**. Filter contents (`client_id: secret-XYZ`, internal hostnames) do not sit on disk in cleartext.
4. **Canonical-form stability**. `{"a":1,"b":2}` and `{"b":2,"a":1}` produce the same key, so JSON cosmetic differences in the live config do not silently fork a checkpoint.
5. **Collision impossibility**. Length-prefix framing rules out the class of attacks where an attacker chooses field contents that mimic the framing of another field.

### Why TWO version numbers, not one?

The file format and the hash-input format are **orthogonal axes of change**:

| Scenario | Bumps `version`? | Bumps `key_format_version`? |
| --- | --- | --- |
| Add `last_observed_at` to `Checkpoint` | Yes | No |
| Add schema fingerprint to hash inputs | No | Yes |
| Change `checkpoints` from map to array | Yes | No |
| Tighten URL normalisation rules | No | Yes |

Collapsing them into one number would mean any tiny file-shape change invalidates every cursor on disk (because old keys would suddenly look wrong-version), and any tiny hash change forces a full file migration. Separating them lets each dimension evolve on its own schedule.

## The monotonic-merge rule

Every successful `put` to the file store runs through a merge step against a fresh re-read of disk, holding an exclusive cross-process lock:

1. **Disk's sequence is greater than or equal to candidate's**: disk wins. (No checkpoint ever moves backwards, whether the higher value came from another process or from this handle's own earlier put.)
2. **Key on disk but not in candidate and not in pending deletes**: preserve disk. (A sibling process's write this handle has never seen.)
3. **Key in pending deletes**: honoured only if disk's value still matches what was observed at delete time. Otherwise the delete is suppressed.

The strict-monotonic rule is the reason hand-editing the file down is silently ignored: the next `put` sees the on-disk value (your edit), sees the candidate (a fresh in-memory cursor), and keeps whichever is higher. For a fresh process opening an existing store, "pre-state" equals "disk", so a regression is still a regression.

## Atomic write protocol

Each write:

1. Encodes the merged state to JSON.
2. Writes it to a temp file in the same directory as the target.
3. `fsync`s the temp file.
4. Atomically renames the temp file over the target (POSIX `rename`, or `MoveFileExW` with `MOVEFILE_WRITE_THROUGH | MOVEFILE_REPLACE_EXISTING` on Windows).
5. `fsync`s the parent directory (POSIX only).

A `kill -9` mid-write cannot corrupt the existing file; the worst case is that the temp file remains on disk and the next write overwrites it.

## Async cancellation hazard

`put` and `delete` on the file store are **not** cancellation-safe. If the future returned by either is dropped after the underlying blocking task has started but before the in-memory install completes, the disk and in-memory state diverge: disk reflects the new value, memory reflects the old one. This handle then returns stale data on subsequent `get` calls until it is dropped and re-opened. The CLI binary drives both calls to completion and does not race them against `select!` arms; users of the library should do the same.
