# Log trigger

Appends each notification as one line of compact NDJSON to a user-specified file. The shape matches what the [echo](./echo.md) trigger emits in pipe mode, so a log file is interchangeable with `aviso listen > notifications.ndjson` output.

## YAML

```yaml
triggers:
  - type: log
    path: /var/log/aviso/mars.log    # required
    retries: 0                       # optional, default 0
    required: true                   # optional, default true
```

## Behavior

- The file is opened with `append(true).create(true)` on the FIRST notification, then held open for the trigger's lifetime. There is no per-write `fsync`; the kernel buffers writes per its standard policy.
- The parent directory must exist. The log trigger does **not** create directories.
- The path is not template-rendered; it's a static string from the YAML.
- The file handle is RAII-closed when the listener stops (Ctrl+C, max-duration reached, supervisor shutdown).
- No rotation. If you need rotation, use `logrotate` with a `copytruncate` strategy and the daemon-side OS-level guarantees about appended writes.

Each line is one notification serialised via `serde_json::to_string(&notification)`. The whole-line write is performed in a single `write_all` call against the locked file handle, so concurrent triggers (or other writers using `append(true)`) cannot interleave mid-line on POSIX filesystems.

## Failure modes

| Error | Surfaces as | Operator action |
|---|---|---|
| Parent directory does not exist | `TriggerError::Io` with `No such file or directory` at first dispatch | Create the parent: `mkdir -p $(dirname /path/to/log)` |
| Path is not writable by the aviso user | `TriggerError::Io` with `Permission denied` at first dispatch | `chown` / `chmod` the parent + file |
| Disk full | `TriggerError::Io` with `No space left on device` at write time | Free space; logs are usually the symptom of a real problem |
| Broken pipe / file vanished | `TriggerError::Io` at write time | `ls -ld $(dirname /path/to/log)` to verify the directory still exists |

The CLI surfaces these via a specific operator-facing hint:

```text
Error in listener my-listener: trigger log(/var/log/aviso/mars.log) failed: io: No such file or directory (os error 2)
  Hint: log trigger could not open `/var/log/aviso/mars.log`: No such file or directory (os error 2). Common causes: the parent directory does not exist (the log trigger does NOT create directories), or the path is not writable by the aviso user. Verify the parent exists and the user can write to it: `ls -ld $(dirname '/var/log/aviso/mars.log')`
  Other listeners continue.
```

## At-least-once and idempotency

The log trigger advances the resume cursor only after a write succeeds. A listener crash mid-write (rare) leaves the cursor un-advanced; on restart the listener redelivers and the same notification's NDJSON line is appended a second time.

Downstream log processors should be ready for this: filter on `event_type@sequence` uniqueness if exact-once semantics are required.

## When to use

- Local persistence: every notification on disk for later analysis.
- Audit trails: append-only file with file-level permissions enforcing read-only access for audit consumers.
- Batch processing pipelines: read the file with any NDJSON-aware tool (`jq -s`, Pandas `read_json(lines=True)`, etc.).

## When NOT to use

- High-volume sustained writes: the trigger does not rotate; the file grows unboundedly. Use `logrotate`.
- Multi-host aggregation: the trigger writes to a single local path. Use [`webhook`](./webhook.md) or [`post`](./post.md) to forward to a central log collector.
- Strict-once semantics: log appends are at-least-once. Use a deduplicating downstream consumer keyed on `event_type@sequence`.
