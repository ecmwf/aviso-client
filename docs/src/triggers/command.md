<!--
SPDX-FileCopyrightText: 2026 European Centre for Medium-Range Weather Forecasts (ECMWF)
SPDX-License-Identifier: Apache-2.0
-->

# Command trigger

<div class="trigger-guide">

Spawns `/bin/sh -c <rendered>` per notification, with the notification's fields
exposed as `AVISO_*` environment variables. Useful for any operator task that
fits in a shell command. **Unix only** (`#[cfg(unix)]`).

## YAML

```yaml
triggers:
  - type: command
    command: "echo {{ notification.event_type }}@{{ notification.sequence }} >> /tmp/seen.log"
    env:                                         # optional; values are literal (NOT templated)
      DATA_DIR: /var/lib/aviso/data
      MODE: production
    working_dir: /var/lib/aviso                  # optional
    timeout: 30s                                 # optional
    retries: 2                                   # optional, default 0
    required: true                               # optional, default true
    fail_fast: true                              # optional, default true
```

**`env:` values are passed literally** to the child process; they are NOT run
through the template engine. Only the `command:` string is templated. If you
need a value that depends on a notification field or another env var, render it
inside the command string (e.g.
`command: "FOO={{ notification.event_type }} ./run.sh"`) or compute it inside
the shell command itself (`command: "DATA=$HOME/aviso ./run.sh"`).

## Template rendering on the command string

The `command:` value runs through the [template engine](./template-engine.md).
Two namespaces:

- `{{ notification.<dotted.path> }}` - substitutes a notification field.
- `{{ env.<NAME> }}` - substitutes a process environment variable.

Example:

```yaml
command: "curl -X POST https://api.example/notify -d '{{ notification.payload }}' -H 'Authorization: Bearer {{ env.API_TOKEN }}'"
```

## Environment variables injected by the dispatcher

Every command runs with these `AVISO_*` env vars set automatically:

| Variable | Value |
|---|---|
| `AVISO_EVENT_TYPE` | The event type (e.g. `mars`) |
| `AVISO_SEQUENCE` | The sequence number (decimal string) |
| `AVISO_IDENTIFIER_<KEY>` | One per identifier field |
| `AVISO_PAYLOAD_JSON` | The payload as compact JSON |
| `AVISO_NOTIFICATION_JSON` | The whole notification as compact JSON (matches the echo trigger's pipe-mode output) |

In `AVISO_IDENTIFIER_<KEY>`, `<KEY>` is uppercased with non-alphanumerics
replaced by `_`. For example, `class` becomes `AVISO_IDENTIFIER_CLASS`.

Operator-supplied `env:` keys are applied **after** the dispatcher-injected
vars, so user keys override dispatcher keys when both are present.

This means you can write commands as either:

```yaml
# Style A: template engine
command: "ingest {{ notification.event_type }} {{ notification.sequence }}"

# Style B: env vars (often shorter and easier to escape)
command: "ingest $AVISO_EVENT_TYPE $AVISO_SEQUENCE"
```

Both produce the same result. Style B has the advantage that the command string
is shorter and the shell handles quoting.

## Output capture

Stdout and stderr are captured concurrently into 4 KiB ring buffers. Stdout
content is dropped (per the no-payload-logging discipline); only the captured
byte count reaches DEBUG-level tracing. Stderr tail surfaces in the public
`TriggerError::Command` variant on non-zero exit.

This means: **don't pipe large output from your command**. If you need to
capture stdout, redirect to a file inside the command (`>> /var/log/foo`).

## Timeout and process tree

When `timeout` expires, the dispatcher sends `SIGKILL` to the shell child and
reaps the zombie. **It does NOT propagate the kill to pipelines, backgrounded
jobs, or grandchildren** that survive the shell. Patterns:

- **Safe**: `command: "exec ./my-binary"` - the shell `exec`s the binary,
  replacing itself in place, so the kill reaches the binary directly.
- **Risky**: `command: "./long-running-command &"` - backgrounded job survives
  the kill.
- **Risky**: `command: "tail -f /var/log/foo | grep ERROR"` - `tail` survives
  even after `grep` is killed.

Use `exec` for single-binary commands. For pipelines that need cleanup, write a
wrapper script with a `trap` handler.

## Fail-fast classification

The `fail_fast` setting defaults to `true` (on). Set it to `false` to turn it
off.

| Error | Fail-fast on | Fail-fast off |
|---|---|---|
| Non-zero exit code (`TriggerError::Command`) | terminal | retryable |
| Template render error (`TriggerError::Template`) | terminal | retryable |
| Timeout (`TriggerError::Timeout`) | retryable | retryable |
| I/O error spawning the child (`TriggerError::Io`) | retryable | retryable |

Terminal failures bypass the `retries` budget and the trigger fails immediately.
The lib emits a hint:

```text
Hint: command trigger exited non-zero. Common causes: the rendered command is malformed (check `{{ notification.* }}` substitutions; the rendered command appears in DEBUG-level tracing only), the command is missing or not on PATH (check the shell's behaviour with `/bin/sh -c '<your command>'`), or the command genuinely failed (check the stderr tail above).
```

## Idempotency

Commands run at-least-once. A required command that succeeds twice (because the
listener crashed before the cursor advanced and the notification was
redelivered) must produce the same observable effect both times.

**Idempotent**: `echo X >> file.log` (duplicate lines are usually fine),
`kubectl annotate node ... --overwrite`.

**Not idempotent**: `mail -s "X" admin@example.com` (sends a second email),
`psql -c "INSERT ..."` (creates a duplicate row).

For non-idempotent commands, either:

- Set `required: false` (the cursor advances even on failure, so
  retry-on-restart doesn't fire) - but then the operator misses the side-effect
  on real failures.
- Make the command itself idempotent (e.g.,
  `INSERT ... ON CONFLICT DO NOTHING`).
- Wrap with a deduplicating layer keyed on `event_type@sequence`.

## When to use

- Glue to existing CLI tools / scripts: any command-line workflow benefits.
- Filesystem actions: `cp`, `mv`, `ln -s`, anything triggered by a notification.
- Lightweight integrations where setting up a webhook receiver is overkill.

## When NOT to use

- Cross-platform deployments: command is Unix only. Use
  [`webhook`](./webhook.md) for cross-platform.
- Long-running tasks: command is per-notification, not per-listener-session.
  Spawning a 60-second task per notification at 100 notifications/sec is going
  to break things.
- Sensitive secrets in the command string: the rendered command appears in
  DEBUG-level tracing (e.g. via the `client.trigger.template.render_failed`
  event when template rendering fails); anyone with access to the DEBUG log sees
  the secret. The public `TriggerError::Command` carries only `exit_code` and
  `stderr_tail`, not the command itself, but the command's own stderr can still
  leak secrets it echoed. Pass secrets via `env:` instead (env values are also
  redacted from the trigger's `Debug` impl and never echoed by the dispatcher).

</div>
